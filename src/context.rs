use crate::schema::*;
use async_graphql::{Schema, EmptySubscription};
use std::sync::{Arc, Mutex}; //might be a better idea to use tokio::Mutex, since it is non blocking, depends on contention really
use std::error::Error;
use std::default::Default;
use sqlx::postgres::{PgPoolOptions, Postgres};
use sqlx::Pool;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use dotenv;
use std::task::{Waker, Poll, Context};
use std::future::Future;
use std::pin::Pin;
use redis::AsyncCommands;


pub type DBClient = Pool<Postgres>;
type InitResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

//REDIS CLIENT
use std::sync::atomic::AtomicI32;
use redis::{RedisFuture, Cmd, Pipeline};
use crate::analytics::{AnalyticsClient, AnalyticsOptions};

const MAX_CONNECTIONS : usize= 3;

struct RedisConnectionFutureInternal {
    conn: Option<redis::aio::Connection>,
    waker: Option<Waker>,
}

#[derive(Clone)]
struct RedisConnectionFuture {
    internal: Arc<Mutex<RedisConnectionFutureInternal>>,
}


pub struct RedisClientPool {
    connections: Vec<redis::aio::Connection>,
    futures: Vec<RedisConnectionFuture>,
}



/*impl Send for RedisClientPool {}*/

pub struct RedisClient {
    pool: Arc<Mutex<RedisClientPool>>,
}

pub struct RedisConnection {
    pool: Arc<Mutex<RedisClientPool>>,
    conn: Option<redis::aio::Connection>,
}

impl redis::aio::ConnectionLike for RedisConnection {
    fn req_packed_command<'a>(&'a mut self, cmd: &'a Cmd) -> RedisFuture<'a, redis::Value> {
        self.conn.as_mut().unwrap().req_packed_command(cmd)
    }

    fn req_packed_commands<'a>(&'a mut self, cmd: &'a Pipeline, offset: usize, count: usize) -> RedisFuture<'a, Vec<redis::Value>> {
        self.conn.as_mut().unwrap().req_packed_commands(cmd, offset, count)
    }

    fn get_db(&self) -> i64 {
        self.conn.as_ref().unwrap().get_db()
    }
}

/*
impl RedisClient {
    /// Get the value of a key.  If key is a vec this becomes an `MGET`.
    async fn get<K: redis::ToRedisArgs>(&self, key: K) {
        self.conn().await.get(key).await
    }

    async fn set<K: redis::ToRedisArgs, V: redis::ToRedisArgs>(&self, key: K, value: V) {
        self.conn().await.set(key, value).await
    }
}*/


impl Drop for RedisConnection {
    fn drop(&mut self) {
        self.pool.lock().unwrap().release(self.conn.take().unwrap())
    }
}



impl Unpin for RedisConnectionFuture {}

//this is way to allocation heavy!
impl Future for RedisConnectionFuture {
    type Output = redis::aio::Connection;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let shared = &mut self.internal.lock().unwrap();

        match shared.conn.take() {
            Some(conn) => Poll::Ready(conn),
            None => {
                shared.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl RedisClientPool {
    fn release(&mut self, conn: redis::aio::Connection) {
        if let Some(future) = self.futures.pop() {
            let internal = &mut future.internal.lock().unwrap();

            internal.conn = Some(conn);
            if let Some(waker) = internal.waker.take() {
                waker.wake()
            }
        } else {
            self.connections.push(conn);
        }
    }

    fn acquire(&mut self) -> RedisConnectionFuture {
        match self.connections.pop() {
            Some(conn) => RedisConnectionFuture{ internal: Arc::new(Mutex::new(RedisConnectionFutureInternal { //todo allocates for no reason!
                waker: None,
                conn: Some(conn)
            }))},
            None => {
                let future = RedisConnectionFuture{ internal: Arc::new(Mutex::new(RedisConnectionFutureInternal { conn: None, waker: None }))};
                self.futures.push(future.clone());
                future
            }
        }
    }
}

impl RedisClient {
    pub async fn pool(client: redis::Client, num_connections: usize) -> redis::RedisResult<RedisClient> {
        let mut connections = Vec::with_capacity(num_connections);

        for _ in 0..num_connections {
           connections.push(client.get_async_connection().await?);
        }

        Ok(RedisClient{
            pool: Arc::new(Mutex::new(RedisClientPool{ connections, futures: Vec::new()})),
        })
    }

    pub async fn conn(&self) -> RedisConnection {
        let acquire = self.pool.lock().unwrap().acquire();

        RedisConnection { pool: self.pool.clone(), conn: Some(acquire.await) }
    }
}

pub struct SharedContext {
    pub db: DBClient,
    pub redis: RedisClient,
    pub schema: APISchema,
    pub analytics: AnalyticsClient,
}


pub fn get_shared<'a>(ctx: &'a async_graphql::Context<'_>) -> &'a SharedContext {
    &ctx.data::<Arc<SharedContext>>().unwrap()
}

pub fn get_db<'a>(ctx: &'a async_graphql::Context<'_>) -> &'a DBClient {
    &get_shared(ctx).db
}

pub fn get_analytics<'a>(ctx: &'a async_graphql::Context<'_>) -> &'a AnalyticsClient {
    &get_shared(ctx).analytics
}

//make number of connections an environment variable
async fn make_db() -> Result<DBClient, sqlx::Error> {
    let url = match dotenv::var("DATABASE_URL") {
        Ok(url) => url,
        Err(e) => return Err(sqlx::Error::Configuration(Box::from(e))),
    };

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_timeout(std::time::Duration::from_millis(100)) //todo: this should not be necessary
        .connect(&url).await?;

    Ok(pool)
}

async fn make_redis() -> InitResult<RedisClient> {
    let url = dotenv::var("REDIS_URL")?;

    let client = redis::Client::open(url)?;
    Ok(RedisClient::pool(client, 5).await?)
}


async fn make_analytics(db: &DBClient, redis: &RedisClient) -> InitResult<AnalyticsClient> {
    AnalyticsOptions::new(db, redis)
        .max_pending_tasks(10)
        .num_workers(1)
        .create().await
}

pub async fn make_shared_context() -> InitResult<Arc<SharedContext>> {
    let db = make_db().await?;
    let redis = make_redis().await?;
    let analytics = make_analytics(&db, &redis).await?;

    Ok(Arc::new(SharedContext {
        db, redis, analytics,
        schema: Schema::build(QueryRoot::default(), MutationRoot::default(), EmptySubscription).finish(),
    }))
}

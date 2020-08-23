use crate::schema::*;
use async_graphql::{Schema, EmptySubscription};
use std::sync::Arc;
use std::default::Default;
use sqlx::postgres::{PgPoolOptions, Postgres};
use sqlx::Pool;
use dotenv;

pub type Client = Pool<Postgres>;

pub struct SharedContext {
    pub db: Client,
    pub schema: APISchema,
}

async fn make_db() -> Result<Pool<Postgres>, sqlx::Error> {
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

pub fn get_shared<'a>(ctx: &'a async_graphql::Context<'_>) -> &'a SharedContext {
    &ctx.data::<Arc<SharedContext>>().unwrap()
}

pub fn get_db<'a>(ctx: &'a async_graphql::Context<'_>) -> &'a Client {
    &get_shared(ctx).db
}

pub async fn make_shared_context() -> Result<Arc<SharedContext>, sqlx::Error> {
    Ok(Arc::new(SharedContext {
        db: make_db().await?,
        schema: Schema::build(QueryRoot::default(), MutationRoot::default(), EmptySubscription).finish(),
    }))
}

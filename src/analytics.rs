use async_graphql_derive::*;
use async_graphql::{Context, FieldResult};
use sqlx::{query, query_as, Postgres};
use chrono::{Utc, DateTime};
use std::time::Duration;
use std::collections::HashMap;
use conc::mpmc::*;
use conc::dataloader::ID;
use serde::{Serialize, Deserialize};
use std::error::Error;
use log::{info, error};
use redis_serialize::RedisValue;


pub struct PostID(ID);
pub struct BondID(ID);
pub struct ProjectID(ID);
pub struct CommentID(ID);

#[Enum]
#[derive(Hash, Serialize, Deserialize)]
pub enum ItemKind {
    Post,
    Bond,
    Project,
    Comment,
}

#[InputObject]
#[derive(Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsItem {
    kind: ItemKind,
    id: ID,
}

#[Enum]
#[derive(Serialize, Deserialize)]
pub enum Action {
    Viewed
}

type Millis = i32;

#[InputObject]
#[derive(Serialize, Deserialize, RedisValue)]
struct AnalyticsEvent {
    action: Action,
    item: AnalyticsItem,
    timestamp: DateTime<Utc>,
    duration: Millis,
}

pub struct DeviceString(String);

pub enum OperatingSystem {
    Win32,
    MacOS,
    Linux,
}

pub struct OperatingSystemInfo {
    system: OperatingSystem,
    version: Version
}

pub struct Version {
    major: u32,
    minor: u32,
    patch: u32
}

pub enum Browser { Safari, Chrome, Edge }

pub struct BrowserInfo {
    browser: Browser,
    version: Version
}

pub struct AppInfo {
    version: Version,
}

pub enum PlatformInfo { Browser(BrowserInfo), App(AppInfo) }

struct Device {
    id: DeviceString,
    platform: PlatformInfo,
    operating_system: OperatingSystemInfo,
}

struct Location {

}

struct AnalyticsInfo {
    views: u64,
    unique_visitors: u64,
    watch_time: Duration,
}

/*
//todo maybe SOA would be be more performant
pub struct Analytics {
    items: HashMap<Item, AnalyticsInfo>,

}

impl Analytics {
    fn analyze(timeline: &Timeline) {
        for (id, page) in &timeline.visited {

        }
    }
}*/

pub struct AnalyticsReport {

}

use redis;
use crate::context::{RedisConnection, get_shared, RedisClient, get_analytics};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use redis::{Value, RedisResult, AsyncCommands};

/*
pub struct Page {
    events: Vec<Event>,
    duration: Duration,
}
*/
pub struct Timeline {
    visited: HashMap<ID, Page>,
    current: ID,
}
/*
impl Event {
    fn from(action: Action) -> Event { Event{ action, timestamp: Utc::now() } }
}

impl Timeline {
    fn navigate_to(&mut self, page: ID) {
        let event = Event::from(Action::NavigatedTo(page));

        match self.visited.get_mut(&page) {
            Some(page) => {page.events.push(event);},
            None => {self.visited.insert( page, Page{
                events: vec![event],
                duration: Duration::from_millis(0),
            });}
        };
    }

    fn current_mut(&mut self) -> &mut Page { self.visited.get_mut(&self.current).unwrap() }

    fn register_event(&mut self, event: Event) {
        self.current_mut().events.push(event);
    }

    fn register_action(&mut self, action: Action) {
        self.register_event(Event::from(action));
    }



}*/



#[derive(Serialize, Deserialize)]
pub enum PageID {
    Detail(AnalyticsItem),
    Home,
    Explore,
}

#[derive(Serialize, Deserialize)]
pub struct Page {
    id: PageID,
    timestamp: DateTime<Utc>,
}

pub enum Task {
    BeginSession(String, Page),
    RegisterEvents(String, Vec<AnalyticsEvent>),
    RegisterEventsThenNavigateTo(String, Vec<AnalyticsEvent>, Page),
}

pub struct AnalyticsClient {
    sender: Sender<Task>,
}

/*
fn hash(value: &str) -> ID {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
*/

pub struct AnalyticsOptions<'a> {
    db_conn_pool: &'a sqlx::PgPool,
    redis_conn_pool: &'a RedisClient,
    num_workers: usize,
    max_pending_tasks: usize,
}

impl<'a> AnalyticsOptions<'a> {
    pub fn new(db: &'a sqlx::PgPool, redis: &'a RedisClient) -> AnalyticsOptions<'a> {
        AnalyticsOptions {
            db_conn_pool: db,
            redis_conn_pool: redis,
            num_workers: 1,
            max_pending_tasks: 10
        }
    }

    pub fn num_workers(mut self, workers: usize) -> Self {
        self.num_workers = workers;
        self
    }

    pub fn max_pending_tasks(mut self, pending: usize) -> Self {
        self.max_pending_tasks = pending;
        self
    }

    pub async fn create(self) -> Result<AnalyticsClient, Box<dyn Error + Send + Sync>> {
        let (receiver, sender) = channel(self.max_pending_tasks);

        for i in 0..self.num_workers {
            let worker = Worker{
                tasks: receiver.clone(),
                db_conn: self.db_conn_pool.acquire().await?,
                redis_conn: self.redis_conn_pool.conn().await
            };

            tokio::spawn(worker.run());
        }

        Ok(AnalyticsClient{ sender })
    }
}

impl AnalyticsClient {
    pub async fn send_task(&self, task: Task) {
        self.sender.clone().send(task).await;
    }

    pub async fn register_events(&self, token: String, events: Vec<AnalyticsEvent>) {
        self.send_task(Task::RegisterEvents(token, events)).await;
    }

    pub async fn begin_session(&self, token: String, page: PageID) {
        self.send_task(Task::BeginSession(token, Page{
            id: page,
            timestamp: Utc::now(),
        })).await;
    }

    pub async fn navigate_to_with_events(&self, token: String, page: PageID, events: Vec<AnalyticsEvent>) {
        self.send_task(Task::RegisterEventsThenNavigateTo(token, events, Page{
            id: page,
            timestamp: Utc::now(),
        })).await;
    }

    //pub async fn navigate_to(&mut self, token: String, page: PageID) {
    //    self.navigate_to_with_events(token, page, Vec::new()).await;
    //}

}

pub struct Worker {
    tasks: Receiver<Task>,
    db_conn: PoolConnection<Postgres>,
    redis_conn: RedisConnection,
}

/*
impl Item {
    fn to_redis(&self) -> String {
        match self {
            Item::Bond(id) => format!("bond:{}", id),
            Item::Comment(id) => format!("comment:{}", id),
            Item::Post(id) => format!("post:{}", id),
            Item::Project(id) => format!("project:{}", id),
        }
    }
}

impl Event {
    fn viewed_to_redis(timestamp: DateTime<Utc>, item: &Item) -> String {
        format!("view:{}:{}", item.to_redis(), timestamp)
    }
}

impl redis::ToRedisArgs for Page {
    fn write_redis_args<W>(&self, out: &mut W) where
        W: ?Sized + redis::RedisWrite {
        let str = match &self.id {
            PageID::Detail(item) => format!("detail:{}:{}", item.to_redis(), self.timestamp),
            PageID::Explore => format!("explore:{}", self.timestamp),
            PageID::Home => format!("home:{}", self.timestamp),
        };

        str.write_redis_args(out);
    }
}

impl redis::FromRedisValue for Page {
    fn from_redis_value(v: &Value) -> RedisResult<Self> {
        let str = String::from_redis_value(v)?;
        let parts = str.split(":");

        match parts.next() {
            Ok(value) => match value {
                "detail" => {},
                "explore" => {},
                "home" => {},
            }
            None => Err(redis::ErrorKind::TypeError),
        }

    }
}*/


#[derive(Serialize, Deserialize, RedisValue)]
struct PageInfo {
    previous: Option<ID>,
    next: Option<ID>,
    page: PageID
}

fn page_by_id(token: &str, id: ID) -> String {
    format!("analytics:timeline:{}:{}", token, id)
}

fn page_events_key(token: &str, id: ID) -> String {
    format!("{}:events", page_by_id(token, id))
}

fn page_info(token: &str, id: ID) -> String {
    format!("{}:info", page_by_id(token, id))
}

fn current_page(token: &str) -> String {
    format!("analytics:timeline:{}:current", token)
}

impl Worker {
    //todo could make this more data oriented by splitting []Event, into []ViewedBond, []ViewedPost, etc
    //however the benefits may be negible due to redis network transfer speeds
    pub async fn run(mut self) {
        loop {
            let task = self.tasks.recv().await;

            if let Err(e) = self.perform_task(task).await {
                error!("Analytics error: {}", e)
            }
        }
    }

    async fn perform_task(&mut self, task: Task) -> Result<(), Box<dyn Error>> {
        match task {
            Task::BeginSession(token, page) => {
                let info = PageInfo{previous: None, next: None, page: page.id};

                redis::pipe()
                    .set(page_info(&token, 0), serde_json::to_string(&info)?)
                    .set(current_page(&token), 0)
                    .query_async(&mut self.redis_conn).await?;
            },
            Task::RegisterEvents(token, events) => {
                let current : ID = self.redis_conn.get(current_page(&token)).await?;
                let mut pipe = redis::pipe();

                self.register_events_and_execute(&mut pipe, &page_events_key(&token, current), &events).await?;
            },
            Task::RegisterEventsThenNavigateTo(token, events, page) => {
                let current_page_key = current_page(&token);
                let previous_id : ID = self.redis_conn.get(&current_page_key).await?;

                let previous_page_key = page_info(&token, previous_id);
                let previous : PageInfo = self.redis_conn.get(&previous_page_key).await?;
                let current_id = previous_id + 1; //could cause problems if multiple workers execute the same cmd
                let current = PageInfo{ previous: Some(previous_id), next: None, page: page.id };
                let previous = PageInfo{ previous: previous.previous, page: previous.page, next: Some(current_id) };

                let mut pipeline = redis::pipe();
                pipeline
                    .set(&previous_page_key, &previous)
                    .set(page_info(&token, current_id), &current)
                    .set(current_page_key, &current);

                self.register_events_and_execute(&mut pipeline, &page_events_key(&token, previous_id), &events).await?;
            }
        };

        Ok(())
    }

    async fn register_events_and_execute(&mut self, pipe: &mut redis::Pipeline, page_events_key: &str, events: &[AnalyticsEvent]) -> Result<(), Box<dyn Error>> {
        for event in events {
            let json = serde_json::to_string(&event)?;
            pipe.lpush(page_events_key, json);
        }

        pipe.query_async(&mut self.redis_conn).await?;
        Ok(())
    }

    //async fn register_events(pipe: &mut redis::Pipeline, page_events_key: &str, event: &[Event]) {}
    //async fn navigated_to(pipe: &mut redis::Pipeline, token: &str, current_page: ID, item: Item) {}
}



use crate::context::SharedContext;
use hyper::{Request, Body};
use crate::auth::get_auth;
use sqlx::pool::PoolConnection;

#[derive(Default)]
pub struct Analytics;

async fn navigate_to(ctx: &Context<'_>, page: PageID, events: Vec<AnalyticsEvent>) -> FieldResult<bool> {
    let token = get_auth(ctx)?.session_token.clone();
    get_analytics(ctx).navigate_to_with_events(token, page, events).await;
    Ok(true)
}

fn get_session_token(ctx: &Context<'_>) -> FieldResult<String> {
    Ok(get_auth(ctx)?.session_token.clone())
}

#[Object]
impl Analytics {
    async fn register_events(&self, ctx: &Context<'_>, events: Vec<AnalyticsEvent>) -> FieldResult<bool> {
        let token = get_session_token(ctx)?;
        get_analytics(ctx).register_events(token, events).await;
        Ok(true)
    }

    async fn begin_session(&self, ctx: &Context<'_>) -> FieldResult<bool> { //somewhat strange
        let token = get_session_token(ctx)?;
        get_analytics(ctx).begin_session(token, PageID::Home).await; //defaults to home page
        Ok(true)
    }

    async fn navigate_to_detail(&self, ctx: &Context<'_>, item: AnalyticsItem, events: Vec<AnalyticsEvent>) -> FieldResult<bool> {
        navigate_to(ctx, PageID::Detail(item), events).await
    }

    async fn navigate_to_home(&self, ctx: &Context<'_>, events: Vec<AnalyticsEvent>) -> FieldResult<bool> {
       navigate_to(ctx,PageID::Home, events).await
    }

    async fn navigate_to_explore(&self, ctx: &Context<'_>, events: Vec<AnalyticsEvent>) -> FieldResult<bool> {
        navigate_to(ctx, PageID::Explore, events).await
    }
}

pub fn event_json_sample() {
    let event = AnalyticsEvent {
        action: Action::Viewed,
        item: AnalyticsItem { kind: ItemKind::Project, id: 0},
        timestamp: Utc::now(),
        duration: 100,
    };

    let json = serde_json::to_string(&event).unwrap();
    println!("Sample json {}", &json);
}
//use crate::context::Context;
use crate::context::{get_db, get_shared};
use crate::dataloaders::*;
use crate::dataloader::ID;
use crate::prof::*;
use crate::auth::MutationAuth;
use crate::chat::{QueryChats, MutationChat};
use async_graphql::{Context, FieldResult, InputValueError, InputValueResult, ScalarType, EmptySubscription, Schema};
use async_graphql_derive::*;
use chrono::{DateTime, Utc};
use std::error::Error;
use std::vec::Vec;
use sqlx::{query_as, query};
use log::info;
use std::default::Default;

pub type Image = i32;

type Timestamp = DateTime<Utc>;

/*
impl<'a> FromSql<'a> for Timestamp {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        let utc = DateTime::<Utc>::from_sql(ty, raw)?;
        Ok(Timestamp(utc.with_timezone(&FixedOffset::east(0))))
    }

    accepts!(TIMESTAMPTZ);
}*/


#[SimpleObject]
#[derive(Clone)]
pub struct Account {
    pub id: i32,
    pub username: String,
    pub profile: String,
}

impl Account {
    pub async fn with_id(ctx: &Context<'_>, id: ID) -> FieldResult<Account> {
        let mut loader = get_loaders(ctx).account.clone();
        Ok(loader.load(id).await?)
        //Account{id: id, username: "".to_string(), profile: "".to_string()}
    }
}

#[derive(Clone)]
pub struct Comment {
    pub id: ID,
    pub account: ID,
    pub mesg: String,
    //pub sent: Timestamp,
}

#[Object]
impl Comment {
    async fn id(&self) -> ID { self.id }
    async fn mesg(&self) -> &str { &self.mesg }
    async fn account(&self, ctx: &Context<'_>) -> FieldResult<Account> {
        let mut account = get_loaders(ctx).account.clone();
        Ok(account.load(self.account).await?)
    }
}

#[derive(Clone)]
pub struct Post {
    pub id: i32,
    pub account: ID,
    pub description: String,
    pub title: String,
    pub image: i32,
}

#[Object]
impl Post {
    async fn id(&self) -> i32 { self.id }
    async fn description(&self) -> &str { &self.description}
    async fn title(&self) -> &str { &self.title }
    async fn image(&self) -> i32 { self.image }
    async fn likes(&self, context: &Context<'_>) -> FieldResult<i32> {
        let db = get_db(context);
        let id: i32 = self.id;

        let row = query!("select count(id) FROM PostLikes where post = $1", id)
            .fetch_one(db)
            .await?;

        Ok(row.count.unwrap() as i32)
    }

    async fn account(&self, context: &Context<'_>) -> FieldResult<Account> {
        //let mut prof = Prof::new();

        /*let result = query_as!(Account, "SELECT id, username, profile FROM Users WHERE id = $1", self.account)
            .fetch_one(get_db(context))
            .await?;*/

        let mut account_loader = get_loaders(context).account.clone();
        //info!("Sending future");
        let result = account_loader.load(self.account).await?;
        //prof.log("Load account");
        Ok(result)
    }

    async fn comments(&self, context: &Context<'_>, cursor: i64, limit: i64) -> FieldResult<Vec<Comment>> {
        let db = get_db(context);
        let id: i32 = self.id;
        let results = query_as!(Comment, "SELECT id, account, mesg FROM Comments WHERE post=$1 LIMIT $2", id, limit)
            .fetch_all(db)
            .await?;

        Ok(results)
    }
}


struct Bond {
    id: ID,
    image: i32,
    sdgs: Vec<i32>,
    title: String,
    issuer: String,
    description: String,
    price: i32,
    interest: f64,
    maturity: i32,
    standardsandpoor: String,
    fitchrating: String,
    cicerorating: String,
    msciesrating: String,
    moodysrating: String,
    amountinvested: i32,
    total: i32,
}

#[async_graphql_derive::Object]
impl Bond {
    async fn id(&self) -> ID { self.id }
    async fn description(&self) -> &str { &self.description }
    async fn price(&self) -> i32 { self.price }
    async fn image(&self) -> i32 { self.image }
    async fn sdgs(&self) -> &[i32] { &self.sdgs }
    async fn name(&self) -> &str { &self.title }
    async fn issuer(&self) -> &str { &self.issuer }
    async fn maturity(&self) -> &i32 { &self.maturity }
    async fn amount_invested(&self) -> i32 { self.amountinvested }
    async fn total(&self) -> i32 { self.total }
    async fn interest(&self) -> f64 { self.interest }
    async fn standardsandpoor(&self) -> &str { &self.standardsandpoor }
    async fn fitchrating(&self) -> &str { &self.fitchrating }
    async fn cicerorating(&self) -> &str { &self.cicerorating }
    async fn msciesrating(&self) -> &str { &self.msciesrating }
    async fn moodysrating(&self) -> &str { &self.moodysrating }
    async fn notable_members(&self, ctx: &Context<'_>) -> FieldResult<Vec<Account>> {
        Ok(vec![])
    }
}

struct Project {
    id: i32,
    name: String,
    description: String,
    image: Image,
    sdgs: Vec<i32>,
    latitude: f64,
    longitude: f64
}
#[Object]
impl Project {
    async fn id(&self) -> i32 { self.id }
    async fn name(&self) -> &str { &self.name }
    async fn description(&self) -> &str { &self.description }
    async fn image(&self) -> i32 { self.image }
    async fn sdgs(&self) -> &[i32] { &self.sdgs }
    async fn latitude(&self) -> f64 { self.latitude }
    async fn longitude(&self) -> f64 { self.longitude }
}

#[Interface(
    field(name = "id", type="ID"),
    field(name = "name", type="&str"),
    field(name = "image", type="Image"),
    field(name = "description", type="&str"),
    field(name = "sdgs", type="&[i32]")
)]
enum ExploreSearchResult {
    Project(Project),
    Bond(Bond),
}

#[derive(Default)]
pub struct QueryFeed;

#[Object]
impl QueryFeed {
    async fn post(&self, ctx: &Context<'_>, id: ID) -> FieldResult<Post> {
        let post = query_as!(Post, "select id, account, image, title, description from Posts where id = $1", id)
            .fetch_one(get_db(ctx))
            .await?;

        Ok(post)
    }

    async fn feed(&self, ctx: &Context<'_>, cursor: i32, limit: i32) -> FieldResult<Vec<Post>> {
        let db = &get_shared(ctx).db;
        let mut prof = Prof::new();

        let posts: Vec<Post> = query_as!(Post,
                "select id, account, image, title, description from Posts",
            )
            .fetch_all(db)
            .await?;

        prof.log("Load feed");

        Ok(posts)
    }
}

#[derive(Default)]
pub struct QueryExplore;

#[Object]
impl QueryExplore {
    async fn explore(&self, ctx: &Context<'_>, sdg: Option<i32>, filter: Option<String>) -> FieldResult<Vec<ExploreSearchResult>> {
        let db = get_db(ctx);

        let mut prof = Prof::new();

        let bonds = query_as!(Bond, "SELECT id, image, sdgs, title, issuer, description,
        interest, maturity, price,
        msciesrating, moodysrating, standardsandpoor,
        fitchrating, amountinvested,
        total, cicerorating
        FROM BONDS
        WHERE
            ($1::text is null or indexed @@ to_tsquery($1))
            AND ($2::int is null or $2 = ANY(sdgs))", filter, sdg)
            .fetch_all(db)
            .await?;

        prof.log("Searched through bonds");

        let projects = query_as!(Project, "SELECT id, name, description, image, sdgs, latitude, longitude \
        FROM Projects
        WHERE
            (indexed @@ to_tsquery($1) or $1 is null)
            AND ($2 = ANY(sdgs) or $2 is null)
            ", filter, sdg)
            .fetch_all(db)
            .await?;

        prof.log("Searched through projects");

        let mut result : Vec<ExploreSearchResult> = Vec::with_capacity(bonds.len() + projects.len());
        for project in projects {
            result.push(ExploreSearchResult::Project(project));
        }

        for bond in bonds {
            result.push(ExploreSearchResult::Bond(bond));
        }

        return Ok(result);
    }
}

//ROOT
#[derive(async_graphql::GQLMergedObject, Default)]
pub struct QueryRoot(pub QueryFeed, pub QueryExplore, pub QueryChats);

#[derive(async_graphql::GQLMergedObject, Default)]
pub struct MutationRoot(pub MutationAuth, pub MutationChat);

/*
pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {}*/

pub type APISchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

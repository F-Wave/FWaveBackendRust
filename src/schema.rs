//use crate::context::Context;
use crate::context::{get_db, get_shared};
use crate::dataloaders::*;
use data::dataloader::ID;
use data::sql_resolve::{SQLResolve, SQLTable};
use data_macros::*;
use data_derive::*;
use crate::prof::*;
use crate::auth::{MutationAuth, get_auth};
use crate::chat::{QueryChats, MutationChat};
use crate::explore::QueryExplore;
use crate::analytics::{QueryAnalytics, MutationAnalytics};
use crate::followers::{QueryFollowers, MutationFollowers};
use async_graphql::{Context, FieldResult, InputValueError, InputValueResult, ScalarType, EmptySubscription, Schema};
use async_graphql_derive::*;
use chrono::{DateTime, Utc};
use std::error::Error;
use std::vec::Vec;
use sqlx::{query_as, query, Row};
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


#[sql("Users")]
pub struct Account {
    pub id: i32,
    pub bio: String,
    pub username: String,
    pub profile: String,
}

/*
impl Account {
    pub async fn with_id(ctx: &Context<'_>, id: ID) -> FieldResult<Account> {
        let mut loader = get_loaders(ctx).account.clone();
        Ok(loader.load(id).await?)
        //Account{id: id, username: "".to_string(), profile: "".to_string()}
    }
}*/

#[sql("Users")]
pub struct MyAccountInfo {
    pub id: ID,
    pub username: String,
    pub profile: String,
    pub bio: String,
    pub email: String,
    pub residence: String,
}

#[derive(async_graphql::GQLMergedObject)]
pub struct MyAccount(pub MyAccountInfo, pub QueryFollowers);

#[derive(Default)]
pub struct QueryMyAccount;



#[Object]
impl QueryMyAccount {
    async fn my_account(&self, ctx: &Context<'_>) -> FieldResult<MyAccount> {
        let user = get_auth(ctx)?.user;

        let info = select_one_from!(ctx, MyAccountInfo, "WHERE id = $1", user);

        //let info = query_one_as!(ctx, MyAccountInfo, "SELECT id, username, profile, bio, email, residence FROM Users WHERE id = $1", user);

        Ok(MyAccount(info, QueryFollowers{user}))
    }
}

#[sql("Comments")]
pub struct Comment {
    pub id: ID,
    pub account: ID,
    pub mesg: String,
    pub sent: DateTime<Utc>,
}

#[sql("Post")]
pub struct Post {
    pub id: i32,
    pub account: ID,
    pub description: String,
    pub title: String,
    pub image: i32,
}

#[Object]
impl Post {
    pub async fn id(&self) -> i32 { self.id }
    pub async fn description(&self) -> &str { &self.description}
    pub async fn title(&self) -> &str { &self.title }
    pub async fn image(&self) -> i32 { self.image }
    pub async fn likes(&self, context: &Context<'_>) -> FieldResult<i32> {
        let db = get_db(context);
        let id: i32 = self.id;

        let row = query!("select count(id) FROM PostLikes where post = $1", id)
            .fetch_one(db)
            .await?;

        Ok(row.count.unwrap() as i32)
    }

    pub async fn account(&self, context: &Context<'_>) -> FieldResult<Account> {
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

    pub async fn comment_count(&self, context: &Context<'_>) -> FieldResult<i64> {
        let results = query!("SELECT COUNT(id) FROM Comments WHERE post=$1", self.id)
            .fetch_one(get_db(context))
            .await?;

        Ok(results.count.unwrap_or_else(|| 0))
    }

    pub async fn comments(&self, context: &Context<'_>, cursor: i64, limit: i64) -> FieldResult<Vec<Comment>> {
        let db = get_db(context);
        let id: i32 = self.id;
        let results = query_as!(Comment, "SELECT id, account, mesg, sent FROM Comments WHERE post=$1 LIMIT $2", id, limit)
            .fetch_all(db)
            .await?;
        Ok(results)
    }
}


pub struct Bond {
    pub id: ID,
    pub image: i32,
    pub sdgs: Vec<i32>,
    pub title: String,
    pub issuer: String,
    pub description: String,
    pub price: i32,
    pub interest: f64,
    pub maturity: i32,
    pub standardsandpoor: String,
    pub fitchrating: String,
    pub cicerorating: String,
    pub msciesrating: String,
    pub moodysrating: String,
    pub amountinvested: i32,
    pub total: i32,
}

#[async_graphql_derive::Object]
impl Bond {
    pub async fn id(&self) -> ID { self.id }
    pub async fn description(&self) -> &str { &self.description }
    pub async fn price(&self) -> i32 { self.price }
    pub async fn image(&self) -> i32 { self.image }
    pub async fn sdgs(&self) -> &[i32] { &self.sdgs }
    pub async fn title(&self) -> &str { &self.title }
    pub async fn issuer(&self) -> &str { &self.issuer }
    pub async fn maturity(&self) -> &i32 { &self.maturity }
    pub async fn amount_invested(&self) -> i32 { self.amountinvested }
    pub async fn total(&self) -> i32 { self.total }
    pub async fn interest(&self) -> f64 { self.interest }
    pub async fn standards_and_poor_rating(&self) -> &str { &self.standardsandpoor }
    pub async fn fitch_rating(&self) -> &str { &self.fitchrating }
    pub async fn cicero_rating(&self) -> &str { &self.cicerorating }
    pub async fn mscies_rating(&self) -> &str { &self.msciesrating }
    pub async fn moodys_rating(&self) -> &str { &self.moodysrating }
    pub async fn notable_members(&self, ctx: &Context<'_>) -> FieldResult<Vec<Account>> {
        Ok(vec![])
    }
}

#[SimpleObject]
pub struct ProjectMember {
    id: i32,
    role: i32,
    account: Account,
    joined: DateTime<Utc>
}

pub struct Project {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub image: Image,
    pub sdgs: Vec<i32>,
    pub latitude: f64,
    pub longitude: f64
}
#[Object]
impl Project {
    pub async fn id(&self) -> i32 { self.id }
    pub async fn title(&self) -> &str { &self.name } //todo change database field to title
    pub async fn description(&self) -> &str { &self.description }
    pub async fn image(&self) -> i32 { self.image }
    pub async fn sdgs(&self) -> &[i32] { &self.sdgs }
    pub async fn latitude(&self) -> f64 { self.latitude }
    pub async fn longitude(&self) -> f64 { self.longitude }
    pub async fn posts(&self, ctx: &Context<'_>, cursor: i64, limit: i64) -> FieldResult<Vec<Post>> {
        Ok(query_all_as!(ctx, Post, "select id, account, image, title, description from Posts where project = $1 LIMIT $2", self.id, limit))
    }
    pub async fn members(&self, ctx: &Context<'_>) -> FieldResult<Vec<ProjectMember>> {
        let results = query_all!(ctx, "select ProjectMembers.id, ProjectMembers.joined, ProjectMembers.role,
        Users.id as account_id, Users.username, Users.profile
        FROM ProjectMembers
        INNER JOIN Users on ProjectMembers.account = Users.id
        WHERE project = $1
        ", self.id);

        let members = results.into_iter().map(|member| ProjectMember{
            id: member.id,
            joined: member.joined,
            account: Account{
                id: member.id,
                username: member.username,
                profile: member.profile,
            },
            role: member.role
        }).collect();

        Ok(members)
    }
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

//ROOT
#[derive(async_graphql::GQLMergedObject, Default)]
pub struct QueryRoot(pub QueryFeed, pub QueryExplore, pub QueryChats, pub QueryAnalytics, pub QueryMyAccount);

#[derive(async_graphql::GQLMergedObject, Default)]
pub struct MutationRoot(pub MutationAuth, pub MutationChat, pub MutationAnalytics, pub MutationFollowers);

/*
pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {}*/

pub type APISchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

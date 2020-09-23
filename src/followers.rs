use crate::context::{get_db, get_redis_conn};
use crate::auth::{get_auth};
use sqlx::{query_as, query};
use async_graphql::{FieldResult, Context};
use async_graphql_derive::*;
use crate::schema::Account;
use data::dataloader::ID;
use data::data_macros::*;
use redis::AsyncCommands;
use std::future::Future;

pub struct QueryFollowers {
    pub user: ID,
}

fn follower_count_key(id: ID) -> String { format!("{}:followers:count", id) }
fn following_count_key(id: ID) -> String { format!("{}:following:count", id) }

const FOLLOWER_COUNT_EXPIRY : usize = 60;

#[Object]
impl QueryFollowers {
    //todo move out
    async fn find_accounts(&self, ctx: &Context<'_>, filter: String) -> FieldResult<Vec<Account>> {
        let results = query_all_as!(ctx, Account, "SELECT id, username, profile FROM USERS
	    WHERE
            username LIKE $2 AND
            id != $1
            AND NOT EXISTS (SELECT * FROM Relationships WHERE following = id AND follower = $1)
	    ", self.user, &filter);

        Ok(results)
    }

    async fn followers(&self, ctx: &Context<'_>) -> FieldResult<Vec<Account>> {
        let results = query_all_as!(ctx, Account, "
        SELECT Relationships.follower as id, Users.username, Users.profile
        FROM Relationships
        INNER join Users ON RELATIONSHIPS.follower = Users.id
        WHERE following = $1;
        ", self.user);

        Ok(results)
    }

    async fn follower_count(&self, ctx: &Context<'_>) -> FieldResult<i64> {
        redis_cached!(ctx, &follower_count_key(self.user), FOLLOWER_COUNT_EXPIRY, query_count!(ctx, "SELECT COUNT(following) FROM Relationships where following = $1", self.user))
    }

    async fn following(&self, ctx: &Context<'_>) -> FieldResult<Vec<Account>> {
        let results = query_as!(Account, "
        SELECT Relationships.follower as id, Users.username, Users.profile
        FROM Relationships
        INNER join Users ON RELATIONSHIPS.following = Users.id
        WHERE follower = $1;
        ", self.user)
            .fetch_all(get_db(ctx))
            .await?;

        Ok(results)
    }

    async fn following_count(&self, ctx: &Context<'_>) -> FieldResult<i64> {
        let user = self.user;
        redis_cached!(ctx, &following_count_key(user), FOLLOWER_COUNT_EXPIRY, query_count!(ctx, "SELECT COUNT(follower) FROM Relationships where follower = $1", user))
    }
}

#[derive(Default)]
pub struct MutationFollowers;

#[Object]
impl MutationFollowers {
    async fn follow(&self, ctx: &Context<'_>, account: ID) -> FieldResult<bool> {
        query!("
        INSERT INTO RELATIONSHIPS (follower, following)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        ", get_auth(ctx)?.user, account)
            .fetch_one(get_db(ctx))
            .await?;

        Ok(true)
    }

    async fn unfollow(&self, ctx: &Context<'_>, account: ID) -> FieldResult<bool> {
        query!("DELETE FROM RELATIONSHIPS WHERE follower = $1 and following = $2", get_auth(ctx)?.user, account)
            .fetch_one(get_db(ctx))
            .await?;

        Ok(true)
    }

    async fn remove_follower(&self, ctx: &Context<'_>, account: ID) -> FieldResult<bool> {
        query!("DELETE FROM RELATIONSHIPS WHERE follower = $1 and following = $2", account, get_auth(ctx)?.user)
            .fetch_one(get_db(ctx))
            .await?;

        Ok(true)
    }
}
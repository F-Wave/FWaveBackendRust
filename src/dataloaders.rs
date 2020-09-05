use crate::context::{SharedContext};
use conc::dataloader::*;
use crate::schema::*;
use crate::prof::*;
use log::info;
use async_trait::async_trait;
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::{Duration};
use sqlx;
use sqlx::{query_as};
use serde::export::Formatter;

pub struct Loaders {
    pub account: DataLoaderEndpoint<Account>,
}

fn loader_error<T, E: Error>(results: &mut HashMap<ID, DataResult<T>>, e: E) {
    for (id, value) in results {
        *value = DataResult::Error(e.to_string());
    }
}

struct AccountLoader {}

struct DidNotFindPostError { id: i32 }

impl std::fmt::Display for DidNotFindPostError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Could not find post {}", self.id))
    }
}

impl std::fmt::Debug for DidNotFindPostError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Could not find post {}", self.id))
    }
}

impl std::error::Error for DidNotFindPostError {

}

#[async_trait]
impl DataLoaderHandler<Account, SharedContext> for AccountLoader {
    async fn batch_execute(
        &mut self,
        shared: &SharedContext,
        results: &mut HashMap<ID, DataResult<Account>>,
    ) {
        let mut prof = Prof::new();
        let db = &shared.db;

        let mut ids: Vec<ID> = Vec::with_capacity(results.len());
        for (id, v) in &*results {
            ids.push(*id);
        }

        info!("LOADING BATCH IDS: {:?}", ids);

        let accounts = match query_as!(Account, "SELECT id, username, profile FROM Users WHERE id = ANY($1)", &ids)
            .fetch_all(db)
            .await {
            Ok(r) => r,
            Err(e) => return loader_error(results, e)
        };


        /*

        let rows = match db
            .query(
                "SELECT id, username, profile FROM Users WHERE id in $1",
                &[&ids],
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return loader_error(results, e),
        };*/

        loader_error(results, DidNotFindPostError{id: 0});

        for account in accounts {
            let id = account.id;
            println!("Got back account {}", id);
            *results.get_mut(&id).unwrap() = DataResult::Ok(account);
        }

        prof.log("Account batch loader");
    }
}

pub fn make_loaders(shared: Arc<SharedContext>) -> Arc<Loaders> {
    Arc::new(Loaders {
        account: DataLoader::new(AccountLoader {}, shared, 10, Duration::from_millis(10)),
    })
}

pub fn get_loaders<'a>(ctx: &'a async_graphql::Context<'_>) -> &'a Loaders {
    &ctx.data::<Arc<Loaders>>().unwrap()
}

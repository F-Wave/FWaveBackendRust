use async_graphql_derive::*;
use async_graphql::{FieldResult, FieldError, Context};
use sqlx::{query};
use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;
use std::sync::Arc;
use chrono::{Utc};
use crate::dataloader::ID;
use crate::context::{get_shared, get_db, SharedContext};

pub struct Auth {
    pub user: i32,
    pub session_token: String,
}

pub fn get_auth<'a>(ctx: &'a Context<'a>) -> FieldResult<&'a Auth> {
    match ctx.data::<Auth>() {
        Ok(auth) => Ok(auth),
        Err(_) => Err(FieldError("Authentication is required".to_string(), None))
    }
}

pub async fn auth_token(ctx: &SharedContext, token: &str) -> FieldResult<Auth> {
    let result = query!("SELECT account FROM Sessions where token = $1", token)
        .fetch_one(&ctx.db)
        .await?;

    Ok(Auth{
        user: result.account,
        session_token: token.to_string()
    })
}

//replace with more cryptographically secure alternative
fn create_session_token() -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .collect()
}

//todo: Prevent too many sessions from being generated for repeated logins
async fn begin_session(ctx: &SharedContext, account: ID, device_token: Option<String>) -> FieldResult<LoginResult> {
    let token = create_session_token();

    let signed_in_at = Utc::now();

    query!("INSERT INTO Sessions (token, account, signedInAt)
    VALUES ($1, $2, $3)
    ", token, account, signed_in_at)
        .execute(&ctx.db)
        .await?;

    if let Some(token) = device_token {
        let _ = query!("
        INSERT INTO Devices (deviceToken, account)
        VALUES ($1, $2)
        ON CONFLICT DO NOTHING
        ", token, account)
            .execute(&ctx.db)
            .await?;
    }

    Ok(LoginResult{token, account_id: account})
}

#[derive(Default)]
pub struct MutationAuth;

#[InputObject]
pub struct CreateAccountForm {
    username: String,
    password: String,
    full_name: String,
    email: String,
    residence: String,
    device_token: Option<String>
}

#[SimpleObject]
pub struct LoginResult {
    token: String,
    account_id: ID
}

#[Object]
impl MutationAuth {
    pub async fn login(&self, ctx: &Context<'_>, username: String, password: String, device_token: Option<String>) -> FieldResult<LoginResult> {
        let shared = get_shared(ctx);
        let found_user = query!("SELECT id, passwordhash FROM Users where username=$1", &username)
            .fetch_optional(&shared.db)
            .await?;

        let stored_user = match found_user {
            Some(user) => user,
            None => return Err(FieldError("No such username".to_string(), None))
        };

        let credentials_match = bcrypt::verify(password, &stored_user.passwordhash)?;
        if credentials_match {
            return begin_session(shared, stored_user.id, device_token).await;
        }
        return Err(FieldError("Incorrect password".to_string(), None));
    }

    pub async fn create_account(&self, ctx: &Context<'_>, form: CreateAccountForm) -> FieldResult<LoginResult> {
        let shared = get_shared(ctx);
        let password_hash = bcrypt::hash(form.password, bcrypt::DEFAULT_COST)?;

        let created_account = query!("INSERT INTO Users (username, passwordHash, fullName, email, residence)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT DO NOTHING
        RETURNING id
        ", form.username, password_hash, form.full_name, form.email, form.residence)
            .fetch_optional(get_db(ctx))
            .await?;

        match created_account {
            Some(account) => begin_session(shared, account.id, form.device_token).await,
            None => Err(FieldError("Username already exists".to_string(), None))
        }
    }
}
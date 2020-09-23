use async_graphql_derive::*;
use async_graphql::{Context, FieldError, FieldResult, Lookahead};
use chrono::{Utc, DateTime};
use sqlx::{query, query_as};
use log::{info};
use std::concat;
use data::dataloader::{ID};
use crate::dataloaders::get_loaders;
use crate::context::*;
use crate::auth::{get_auth, Auth};
use crate::schema::Account;


enum ChatRole {
    Owner = 1,
    Admin = 2,
    Member = 3
}

pub struct Group {
    id: ID,
    groupname: String,
    profile: ID,
}

#[Object]
impl Group {
    async fn id(&self) -> ID { self.id }
    async fn name(&self) -> &str { &self.groupname }
    async fn profile(&self) -> ID { self.profile }
    async fn member_count(&self, ctx: &Context<'_>) -> FieldResult<i32> {
        let result = query!("SELECT COUNT(id) FROM GroupMembers WHERE chat = $1", self.id)
            .fetch_one(get_db(ctx))
            .await?;

        Ok(result.count.unwrap_or(0) as i32)
    }

    async fn members(&self, ctx: &Context<'_>) -> FieldResult<Vec<Account>> {
        let accounts = query_as!(Account, "SELECT Users.id, Users.username, Users.profile FROM GroupMembers
        INNER JOIN Users on Users.id = GroupMembers.account
        WHERE chat=$1\

        ", self.id)
            .fetch_all(get_db(ctx))
            .await?;

        Ok(accounts)
    }

    async fn messages(&self, ctx: &Context<'_>) -> FieldResult<Vec<Message>> {
        get_messages(get_db(ctx), self.id).await
    }
}


#[SimpleObject]
struct Message {
    id: ID,
    account: Account,
    mesg: String,
    sent: DateTime<Utc>,
}

async fn get_messages(db: &DBClient, chat: ID) -> FieldResult<Vec<Message>> {
    let messages = query!("SELECT Messages.ID, Users.ID as account_id, Users.username, Users.profile, Messages.mesg, Messages.sent
	FROM Messages
	INNER JOIN Users ON Messages.account = Users.ID
	WHERE Messages.chat= $1", chat)
        .fetch_all(db).await?
        .into_iter()
        .map(|result| Message{
            id: result.id,
            account: Account{
                id: result.account_id,
                username: result.username,
                profile: result.profile
            },
            mesg: result.mesg,
            sent: result.sent
        });

    Ok(messages.collect())
}

pub struct DM {
    id: ID,
    account: Account
}

#[Object]
impl DM {
    async fn id(&self) -> ID { self.id }
    async fn dm(&self) -> &Account { &self.account }
    async fn messages(&self, ctx: &Context<'_>) -> FieldResult<Vec<Message>> {
        get_messages(get_db(ctx), self.id).await
    }
}

#[Interface(
    field(name="id", type="ID")
)]
enum Chat {
    DM(DM),
    Group(Group)
}

#[derive(Default)]
pub struct QueryChats {}

#[Object]
impl QueryChats {
    async fn dm(&self, ctx: &Context<'_>, id: ID) -> FieldResult<DM> {
        let auth = get_auth(ctx)?;
        let db = get_db(ctx);

        if ctx.look_ahead().field("dm").exists() {
            let result =
                query!("SELECT DMS.id, Users.id AS account_id, Users.username, Users.profile FROM DMS
            INNER JOIN Users ON
            (CASE
                WHEN DMS.user1=$1 THEN Users.id = DMS.user2
                ELSE Users.id = DMS.user1
            END) WHERE (user1=$1 or user2=$1) and DMS.id=$2", auth.user, id)
                .fetch_one(db)
                .await?;
            Ok(DM{
                id: id,
                account: Account{
                    id: result.account_id,
                    username: result.username,
                    profile: result.profile
                }
            })
        } else {
            let result = query!("SELECT id FROM DMS WHERE (user1=$1 or user2=$1) and id=$2", auth.user, id)
                .fetch_one(db)
                .await?;

            Ok(DM{
                id: id,
                account: Account{id: 0, username: "".to_string(), profile: "".to_string()}
            })
        }
    }

    async fn group(&self, ctx: &Context<'_>, id: ID) -> FieldResult<Group> {
        let auth = get_auth(ctx)?;

        let group = query_as!(Group, "SELECT id, groupname, profile FROM Groups
        WHERE id = $1 AND EXISTS (SELECT * FROM GroupMembers WHERE
		    chat = Groups.id AND
		    account = $2
		)", id, auth.user)
            .fetch_one(get_db(ctx))
            .await?;

        Ok(group)
    }

    //todo show last used chat first
    async fn chats(&self, ctx: &Context<'_>) -> FieldResult<Vec<Chat>> {
        let auth = get_auth(ctx)?;

        let dms = query!("SELECT DMS.id, Users.id AS account_id, Users.username, Users.profile FROM DMS
        INNER JOIN Users ON
            (CASE
                WHEN DMS.user1=$1 THEN Users.id = DMS.user2
                ELSE Users.id = DMS.user1
            END)
        WHERE user1=$1 or user2=$1", auth.user)
            .fetch_all(get_db(ctx))
            .await?;

        let groups = query_as!(Group, "SELECT id, groupname, profile FROM Groups
        WHERE EXISTS (SELECT * FROM GroupMembers WHERE
		    chat = Groups.id AND
		    account = $1
		)", auth.user)
            .fetch_all(get_db(ctx))
            .await?;

        let mut results = Vec::with_capacity(dms.len() + groups.len());
        for dm in dms {
            results.push(Chat::DM(DM{
                id: dm.id,
                account: Account{
                    id: dm.account_id,
                    username: dm.username,
                    profile: dm.profile
                }
            }));
        }
        for group in groups {
            results.push(Chat::Group(group));
        }

        Ok(results)
    }
}

#[derive(Default)]
pub struct MutationChat {}

#[Object]
impl MutationChat {
    //todo: ensure that user has permission!
    async fn send_message(&self, ctx: &Context<'_>, chat: ID, mesg: String) -> FieldResult<ID> {
        let auth = get_auth(ctx)?;
        let sent = Utc::now();
        let mesg = query!("INSERT INTO Messages (account, chat, mesg, sent)
        VALUES ($1, $2, $3, $4)
        RETURNING ID
        ", auth.user, chat, mesg, sent)
            .fetch_one(get_db(ctx))
            .await?;

        return Ok(mesg.id);
    }


    async fn create_dm(&self, ctx: &Context<'_>, account: ID) -> FieldResult<ID> {
        let auth = get_auth(ctx)?;
        let created = Utc::now();

        let dm = query!("INSERT INTO DMS (user1, user2, created)
        SELECT $1, $2, $3
        WHERE NOT EXISTS (SELECT * FROM DMs WHERE (user1 = $1 AND user2 = $2) or (user1 = $2 AND user2 = $1))
        RETURNING id", auth.user, account, created)
            .fetch_one(get_db(ctx))
            .await?;

        Ok(dm.id)
    }

    async fn create_group(&self, ctx: &Context<'_>, group_name: String, members : Vec<ID>) -> FieldResult<ID> {
        let auth : &Auth = get_auth(ctx)?;
        let db = get_db(ctx);

        let created = Utc::now(); //todo is it better for the user to send the creation time

        let mut pruned_members = members;
        pruned_members.retain(|id| *id != auth.user);
        pruned_members.sort();
        pruned_members.dedup();

        let group = query!("INSERT INTO Groups (groupname, profile, created)
        VALUES ($1, $2, $3)
        RETURNING ID
        ", group_name, 0, created)
            .fetch_one(db)
            .await?;


        let mut sql_statement = "INSERT INTO GroupMembers (account, role, joined, chat)
        VALUES ($1, $2, $4, $5)
        ".to_string();

        for (i, member) in pruned_members.iter().enumerate() {
            sql_statement.push_str(&format!("({}, $3, $4, $5)", member));
            if i+1 < pruned_members.len() {
                sql_statement += ",\n";
            }
        }
        sql_statement += "\nRETURNING id;";

        match sqlx::query(&sql_statement)
            .bind(auth.user)
            .bind(ChatRole::Owner as i32)
            .bind(ChatRole::Member as i32)
            .bind(created)
            .bind(group.id)
            .fetch_one(db)
            .await {
            Ok(_) => Ok(group.id),
            Err(e) => {
                let _ = query!("DELETE FROM Groups where id=$1 RETURNING id", group.id)
                    .fetch_one(db).await?;

                info!("{}", e);
                Err(FieldError("Adding members failed".to_string(), None))
            }
        }


    }
}
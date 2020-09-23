use sqlx::{query_as};

#[macro_export]
macro_rules! redis_cached {
    ($ctx:expr, $key: expr, $expiry: expr, $value: expr) => {
        {
            let mut redis = get_redis_conn($ctx).await;

            match redis.get($key).await {
                Ok(value) => Ok(value),
                Err(_) => {
                    let value = $value;
                    redis.set_ex($key, value, $expiry).await?;
                    Ok(value)
                }
            }
        }
    }
}

//todo create macro, to create macro
#[macro_export]
macro_rules! query_count {
    ($ctx:expr, $sql:expr, $($arg:expr),*) => {
        query!($sql, $($arg),*)
            .fetch_one(get_db($ctx))
            .await?
            .count.unwrap_or_else(|| 0)
    }
}

#[macro_export]
macro_rules! query_one {
    ($ctx:expr, $sql:literal, $($arg:expr),*) => {
        query!($sql, $($arg),*)
            .fetch_one(get_db($ctx))
            .await?
    }
}

#[macro_export]
macro_rules! query_one_as {
    ($ctx:expr, $typ:path, $sql:literal, $($arg:expr),*) => {
        query_as!($typ, $sql, $($arg),*)
            .fetch_one(get_db($ctx))
            .await?
    }
}

#[macro_export]
macro_rules! query_all {
    ($ctx:expr, $sql:literal, $($arg:expr),*) => {
        query!($sql, $($arg),*)
            .fetch_all(get_db($ctx))
            .await?
    }
}

#[macro_export]
macro_rules! query_all_as {
    ($ctx:expr, $typ:path, $sql:literal, $($arg:expr),*) => {
        query_as!($typ, $sql, $($arg),*)
            .fetch_all(get_db($ctx))
            .await?
    }
}

//($ctx:expr, $table: ident, $sql: literal, $($field:ident -> $of_type:path)*) => {
//
//     }

//todo remove duplication
#[macro_export]
macro_rules! select_one_from {
    ($ctx:expr, $typ:path, $sql:literal, $($arg:expr),*) => {
        {
            let ctx = $ctx;
            let sql = <($typ)>::sql_for_select($ctx, $sql);
            let row = sqlx::query(&sql)
                $(.bind($arg))*
                .fetch_one(get_db($ctx))
                .await?;
            let mut index = 0;

            <($typ)>::resolve(&row, ctx, &ctx.look_ahead(), &mut index)?
        }
    }
}

#[macro_export]
macro_rules! select_all_from {
    ($ctx:expr, $typ:path, $sql:literal, $($arg:expr),*) => {
        {
            let ctx = $ctx;
            let sql = <($typ)>::sql_for_select($ctx, $sql);
            let row = sqlx::query(&sql)
                $(.bind($arg))*
                .fetch_all(get_db($ctx))
                .await?;
            let mut index = 0;

            <($typ)>::resolve(&row, ctx, &ctx.look_ahead(), &mut index)?
        }
    }
}
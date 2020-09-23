use async_graphql::{Context, FieldResult, Lookahead};
use sqlx::postgres::PgRow;
use sqlx::Row;

pub trait SQLResolve {
    type T;
    fn resolve(row: &PgRow, ctx: &Context<'_>, look: &Lookahead<'_>, index: &mut usize) -> FieldResult<Self::T>;
    fn require_field(fields: &mut Vec<Field>, joins: &mut Vec<Join>, lookahead: &Lookahead<'_>, field: &Field) {
        fields.push(field.clone());
    }
}

pub trait SQLTable : SQLResolve {
    const TABLE : &'static str;

    fn sql_for_select(ctx: &Context<'_>, where_clause: &str) -> String {
        let mut fields = Vec::new();
        let mut joins = Vec::new();
        Self::require_field(&mut fields, &mut joins, &ctx.look_ahead(), &Field {
            table: Self::TABLE,
            name: "".to_string(),
            field: "",
        });

        if fields.len() == 0 {
            return "".to_string();
        }

        let mut sql = "SELECT ".to_string();
        for (i, field) in fields.iter().enumerate() {
            sql += field.table;
            sql += ".";
            sql += field.field;
            sql += " as ";
            sql += &field.name;
            if i + 1 < fields.len() {
                sql += ", ";
            }
        }

        sql += " FROM ";
        sql += joins[0].table;

        for (i, join) in joins[1..].iter().enumerate() {
            sql += " INNER JOIN ";
            sql += join.table;
            sql += "ON ";
            sql += join.table;
            sql += ".id = ";
            sql += joins[i - 1].table;
            sql += ".";
            sql += join.on;
            sql += "\n";
            joins[i - 1].table;
        }

        sql += " ";
        sql += where_clause;

        println!("Generated sql {}", sql);

        return sql;
    }
}

#[derive(Clone)]
pub struct Field {
    pub table: &'static str,
    pub name: String,
    pub field: &'static str,
}

#[derive(Clone)]
pub struct Join {
    pub table: &'static str,
    pub on: &'static str,
}


macro_rules! sql_resolve_for_prim {
    ($typ: path) => {
        impl SQLResolve for $typ {
            type T = $typ;
            fn resolve(row: &PgRow, ctx: &Context<'_>, look: &Lookahead<'_>, index: &mut usize) -> FieldResult<Self::T> {
                *index += 1;
                Ok(row.try_get(*index - 1)?)
            }
        }
    }
}

sql_resolve_for_prim!(String);
//sql_resolve_for_prim!(str);
sql_resolve_for_prim!(i32);
sql_resolve_for_prim!(u32);
sql_resolve_for_prim!(f32);
sql_resolve_for_prim!(f64);

use crate::context::{get_db};
use crate::auth::{get_auth};
use crate::prof::Prof;
use conc::dataloader::{ID};
use crate::schema::{Project, Bond, Post, Image};
use async_graphql_derive::*;
use async_graphql::{Context, FieldResult};
use sqlx::{query, query_as};



//field(name = "sdgs", type="&[i32]")
#[Interface(
field(name = "id", type="ID"),
field(name = "title", type="&str"),
field(name = "image", type="Image"),
field(name = "description", type="&str"),
)]
enum Content {
    Project(Project),
    Bond(Bond),
    Post(Post),
}

#[derive(Default)]
pub struct QueryExplore;

pub struct Category {
    id: ID,
    title: String,
    description: String,
    image: Image,
}

#[Object]
impl Category {
    async fn id(&self) -> ID { self.id }
    async fn title(&self) -> &str { &self.title }
    async fn description(&self) -> &str { &self.description }
    async fn image(&self) -> Image { self.image }

    async fn trending(&self, ctx: &Context<'_>, cursor: i32, limit: i32) -> FieldResult<Vec<Content>> {
        Ok(vec![])
    }

    async fn top_investments(&self, ctx: &Context<'_>, cursor: i32, limit: i32) -> FieldResult<Vec<Content>> {
        Ok(vec![])
    }
}


fn append_content<F: Fn(T) -> Content, T>(vec: &mut Vec<Content>, f: F, content: Vec<T>) {
    vec.reserve(vec.len() + content.len());
    for elem in content {
        vec.push(f(elem));
    }
}

#[Object]
impl QueryExplore {
    async fn trending(&self, ctx: &Context<'_>, cursor: i32, limit: i32) -> FieldResult<Vec<Content>> {

        Ok(vec![])
    }

    async fn top_investments(&self, ctx: &Context<'_>, cursor: i32, limit: i32) -> FieldResult<Vec<Content>> {
        Ok(vec![])
    }

    async fn categories_for_you(&self, ctx: &Context<'_>, cursor: i32, limit: i32) -> FieldResult<Vec<Content>> {
        Ok(vec![])
    }

    async fn search(&self, ctx: &Context<'_>, sdg: Option<i32>, filter: Option<String>) -> FieldResult<Vec<Content>> {
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

        let mut result : Vec<Content> = Vec::with_capacity(bonds.len() + projects.len());
        append_content(&mut result, Content::Project, projects);
        append_content(&mut result, Content::Bond, bonds);

        return Ok(result);
    }
}
use chrono::{DateTime, Utc};
use async_graphql::{Value, InputValueResult, InputValueError, ScalarType};
use async_graphql_derive::Scalar;
use sqlx::encode::{Encode, Decode, IsNull};
use sqlx::postgres::{Postgres, PgArgumentBuffer};

#[derive(Clone)]
pub struct Timestamp(DateTime<Utc>);

impl Timestamp {
    pub fn now() -> Timestamp {
        Timestamp(Utc::now())
    }
}


#[async_graphql_derive::Scalar]
impl ScalarType for Timestamp {
    fn parse(value: Value) -> InputValueResult<Self> {
        if let Value::String(value) = &value {
            Ok(Timestamp(DateTime::from(DateTime::parse_from_rfc3339(
                value,
            )?)))
        } else {
            Err(InputValueError::ExpectedType(value))
        }
    }

    fn to_value(&self) -> Value {
        Value::String(self.0.to_rfc3339())
    }
}

impl Encode<'_, Postgres> for Timestamp {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> IsNull {

    }
}
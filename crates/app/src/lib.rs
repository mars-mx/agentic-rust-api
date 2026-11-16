pub mod catalog;

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};

#[derive(Debug, Clone)]
pub struct Database {
    pool: PgPool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub organization_id: String,
    pub user_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
}

impl Database {
    pub async fn connect(url: &str) -> Result<Self, Error> {
        Ok(Self {
            pool: PgPoolOptions::new().connect(url).await?,
        })
    }

    pub fn lazy(url: &str) -> Result<Self, Error> {
        Ok(Self {
            pool: PgPoolOptions::new().connect_lazy(url)?,
        })
    }

    async fn begin(&self, scope: &Scope) -> Result<Transaction<'static, Postgres>, Error> {
        let mut transaction = self.pool.begin().await?;
        db::scope(&mut transaction, &scope.organization_id, &scope.user_id).await?;
        Ok(transaction)
    }
}

impl From<PgPool> for Database {
    fn from(pool: PgPool) -> Self {
        Self { pool }
    }
}

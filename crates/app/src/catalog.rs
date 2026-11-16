use domain::catalog::{Item, Title};

use crate::{Database, Error, Scope};

#[tracing::instrument(skip_all)]
pub async fn create_item(database: &Database, scope: &Scope, title: &str) -> Result<Item, Error> {
    let title = Title::parse(title).map_err(|error| Error::Invalid(error.to_string()))?;
    let mut transaction = database.begin(scope).await?;
    let item =
        db::catalog::insert_item(&mut transaction, &scope.organization_id, title.as_str()).await?;
    transaction.commit().await?;
    Ok(item)
}

#[tracing::instrument(skip_all)]
pub async fn list_items(database: &Database, scope: &Scope) -> Result<Vec<Item>, Error> {
    let mut transaction = database.begin(scope).await?;
    let items = db::catalog::list_items(&mut transaction).await?;
    transaction.commit().await?;
    Ok(items)
}

#[tracing::instrument(skip_all)]
pub async fn search_items(
    database: &Database,
    scope: &Scope,
    term: &str,
) -> Result<Vec<Item>, Error> {
    let term = term.trim();
    if term.is_empty() {
        return Err(Error::Invalid(
            "the search term must not be empty".to_owned(),
        ));
    }
    let mut transaction = database.begin(scope).await?;
    let items = db::catalog::search_items(&mut transaction, term).await?;
    transaction.commit().await?;
    Ok(items)
}

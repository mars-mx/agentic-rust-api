use app::catalog::{create_item, list_items, search_items};
use app::{Database, Scope};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

fn scope(organization_id: &str) -> Scope {
    Scope {
        organization_id: organization_id.to_owned(),
        user_id: format!("user_of_{organization_id}"),
    }
}

async fn as_app_api(
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
) -> Result<PgPool, sqlx::Error> {
    pool_options
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query_file!("tests/sql/as_app_api.sql")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(connect_options)
        .await
}

async fn insert_shared(
    pool_options: &PgPoolOptions,
    connect_options: &PgConnectOptions,
) -> Result<(), sqlx::Error> {
    let owner = pool_options
        .clone()
        .connect_with(connect_options.clone())
        .await?;
    sqlx::query_file!("tests/sql/insert_shared_item.sql", "Lamp shared")
        .execute(&owner)
        .await?;
    Ok(())
}

fn titles(items: &[domain::catalog::Item]) -> Vec<&str> {
    items.iter().map(|item| item.title.as_str()).collect()
}

#[sqlx::test(migrations = "../db/migrations")]
#[ignore = "needs Postgres; make db runs it"]
async fn each_organization_sees_its_own_and_the_shared_items_only(
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
) {
    insert_shared(&pool_options, &connect_options)
        .await
        .unwrap();
    let database = Database::from(as_app_api(pool_options, connect_options).await.unwrap());
    let (a, b) = (scope("org_a"), scope("org_b"));
    create_item(&database, &a, "Lamp alpha").await.unwrap();
    create_item(&database, &b, "Lamp beta").await.unwrap();

    let seen_by_a = list_items(&database, &a).await.unwrap();
    assert_eq!(titles(&seen_by_a), ["Lamp alpha", "Lamp shared"]);
    assert_eq!(seen_by_a[1].organization_id, None);
    assert_eq!(
        titles(&list_items(&database, &b).await.unwrap()),
        ["Lamp beta", "Lamp shared"]
    );
    assert_eq!(
        titles(&search_items(&database, &a, "lamp").await.unwrap()),
        ["Lamp alpha", "Lamp shared"]
    );
    assert_eq!(
        titles(&search_items(&database, &b, "beta").await.unwrap()),
        ["Lamp beta"]
    );
    assert_eq!(
        titles(&search_items(&database, &a, "beta").await.unwrap()),
        Vec::<&str>::new()
    );
}

#[sqlx::test(migrations = "../db/migrations")]
#[ignore = "needs Postgres; make db runs it"]
async fn an_organization_cannot_write_a_row_for_another(
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
) {
    let pool = as_app_api(pool_options, connect_options).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    db::scope(&mut transaction, "org_a", "user_of_org_a")
        .await
        .unwrap();
    let error = db::catalog::insert_item(&mut transaction, "org_b", "Lamp")
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501"),
        "{error}"
    );
}

#[sqlx::test(migrations = "../db/migrations")]
#[ignore = "needs Postgres; make db runs it"]
async fn a_query_without_a_scope_fails_instead_of_reading_rows(
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
) {
    insert_shared(&pool_options, &connect_options)
        .await
        .unwrap();
    let pool = as_app_api(pool_options, connect_options).await.unwrap();
    let mut connection = pool.acquire().await.unwrap();
    let error = db::catalog::list_items(&mut connection).await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42704"),
        "{error}"
    );
}

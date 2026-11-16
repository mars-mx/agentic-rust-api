pub fn f() {
    let _query = sqlx::query_as_with::<sqlx::Postgres, (i32,), _>("SELECT 1", sqlx::postgres::PgArguments::default());
}

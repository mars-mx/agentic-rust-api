pub fn f() {
    let _query = sqlx::query_scalar_with::<sqlx::Postgres, i32, _>("SELECT 1", sqlx::postgres::PgArguments::default());
}

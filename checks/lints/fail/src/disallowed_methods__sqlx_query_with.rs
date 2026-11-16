pub fn f() {
    let _query = sqlx::query_with("SELECT 1", sqlx::postgres::PgArguments::default());
}

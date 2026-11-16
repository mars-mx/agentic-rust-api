pub fn f() {
    let _query = sqlx::query_scalar::<sqlx::Postgres, i32>("SELECT 1");
}

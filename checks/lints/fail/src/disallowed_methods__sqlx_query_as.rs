pub fn f() {
    let _query = sqlx::query_as::<sqlx::Postgres, (i32,)>("SELECT 1");
}

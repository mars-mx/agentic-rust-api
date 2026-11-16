pub fn f() {
    let _query = sqlx::query::<sqlx::Postgres>("SELECT 1");
}

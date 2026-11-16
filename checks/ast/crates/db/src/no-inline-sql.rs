async fn f(c: &mut PgConnection) { sqlx::query!("SELECT 1").execute(c).await; }

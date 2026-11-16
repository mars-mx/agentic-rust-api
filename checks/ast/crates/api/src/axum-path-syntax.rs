fn r() -> Router { Router::new().route("/s/:id", get(show)) }

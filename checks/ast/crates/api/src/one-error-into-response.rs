impl IntoResponse for StoreError { fn into_response(self) -> Response { StatusCode::NOT_FOUND.into_response() } }

# TL;DR don't use this

I attempted to port axum-sqlx-tx to sea-orm, but for reasons that are mysterious to me, the middleware
is leaking connections and eventually exhausts the pool.

What I did instead is a relatively simple Axum from_fn_with_state style middleware:

```rust
async fn run_request_bound_transaction<B>(
  mut request: Request<B>,
  next: Next<B>,
  db: &DatabaseConnection,
) -> Result<Response, DbErr> {
  let extensions_mut = request.extensions_mut();
  let tx = db.begin().await?;
  let tx_arc = Arc::new(tx);

  extensions_mut.insert(ConnectionWrapper::DatabaseTransaction(Arc::downgrade(
    &tx_arc,
  )));
  let response = next.run(request).await;
  let tx = Arc::try_unwrap(tx_arc).map_err(|arc| {
    DbErr::Custom(format!(
      "Cannot finish database transaction because it still has {} strong references",
      Arc::strong_count(&arc)
    ))
  })?;

  if response.status().is_success() {
    tx.commit().await?;
  } else {
    tx.rollback().await?;
  }

  Ok(response)
}

pub async fn request_bound_transaction<B>(
  State(db_conn): State<Arc<DatabaseConnection>>,
  request: Request<B>,
  next: Next<B>,
) -> Response {
  run_request_bound_transaction(request, next, &db_conn)
    .await
    .unwrap_or_else(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response())
}
```

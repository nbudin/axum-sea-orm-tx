//! A silly server that generates random numers, but only commits positive ones.

use std::error::Error;

use axum::{response::IntoResponse, routing::get, Json};
use axum_sea_orm_tx::Tx;
use http::StatusCode;
use sea_orm::{ConnectionTrait, Database, DbErr, Statement};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // You can use any sqlx::Pool
    let db = tempfile::NamedTempFile::new()?;
    let pool = Database::connect(&format!("sqlite://{}", db.path().display())).await?;

    // Create a table (in a real application you might run migrations)
    pool.execute(Statement::from_string(
        pool.get_database_backend(),
        "CREATE TABLE IF NOT EXISTS numbers (number INT PRIMARY KEY);".to_string(),
    ))
    .await?;

    // Standard axum app setup
    let app = axum::Router::new()
        .route("/numbers", get(list_numbers).post(generate_number))
        // Apply the Tx middleware
        .layer(axum_sea_orm_tx::Layer::new(pool.clone()));

    let server = axum::Server::bind(&([0, 0, 0, 0], 0).into()).serve(app.into_make_service());

    println!("Listening on {}", server.local_addr());
    server.await?;

    Ok(())
}

async fn list_numbers(tx: Tx) -> Result<Json<Vec<i32>>, DbError> {
    let numbers: Vec<i32> = tx
        .query_all(Statement::from_string(
            tx.get_database_backend(),
            "SELECT * FROM numbers".to_string(),
        ))
        .await?
        .into_iter()
        .map(|res| res.try_get("numbers", "number").unwrap())
        .collect();

    Ok(Json(numbers))
}

async fn generate_number(tx: Tx) -> Result<(StatusCode, Json<i32>), DbError> {
    let number: i32 = tx
        .query_one(Statement::from_string(
            tx.get_database_backend(),
            "INSERT INTO numbers VALUES (random()) RETURNING number;".to_string(),
        ))
        .await?
        .unwrap()
        .try_get("", "number")?;

    // Simulate a possible error – in reality this could be something like interacting with another
    // service, or running another query.
    let status = if number > 0 {
        StatusCode::OK
    } else {
        StatusCode::IM_A_TEAPOT
    };

    // no need to explicitly resolve!
    Ok((status, Json(number)))
}

// An sqlx::Error wrapper that implements IntoResponse
struct DbError(DbErr);

impl From<DbErr> for DbError {
    fn from(error: DbErr) -> Self {
        Self(error)
    }
}

impl IntoResponse for DbError {
    fn into_response(self) -> axum::response::Response {
        println!("ERROR: {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
    }
}

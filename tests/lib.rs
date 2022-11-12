use axum::response::IntoResponse;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement, Value};
use tempfile::NamedTempFile;
use tower::ServiceExt;

type Tx<E = axum_sea_orm_tx::Error> = axum_sea_orm_tx::Tx<DatabaseConnection, E>;

#[tokio::test]
async fn commit_on_success() {
    let (_db, pool, response) = build_app(|mut tx: Tx| async move {
        let (_, name) = insert_user(&mut tx, 1, "huge hackerman").await;
        format!("hello {name}")
    })
    .await;

    assert!(response.status.is_success());
    assert_eq!(response.body, "hello huge hackerman");

    let users: Vec<(i32, String)> = pool
        .query_all(Statement::from_string(
            pool.get_database_backend(),
            "SELECT * FROM users".to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|res| {
            res.try_get_many("", &["id".to_string(), "name".to_string()])
                .unwrap()
        })
        .collect();
    assert_eq!(users, vec![(1, "huge hackerman".to_string())]);
}

#[tokio::test]
async fn rollback_on_error() {
    let (_db, pool, response) = build_app(|mut tx: Tx| async move {
        insert_user(&mut tx, 1, "michael oxmaul").await;
        http::StatusCode::BAD_REQUEST
    })
    .await;

    assert!(response.status.is_client_error());
    assert!(response.body.is_empty());

    assert_eq!(get_users(&pool).await, vec![]);
}

#[tokio::test]
async fn explicit_commit() {
    let (_db, pool, response) = build_app(|mut tx: Tx| async move {
        insert_user(&mut tx, 1, "michael oxmaul").await;
        tx.commit().await.unwrap();
        http::StatusCode::BAD_REQUEST
    })
    .await;

    assert!(response.status.is_client_error());
    assert!(response.body.is_empty());

    assert_eq!(
        get_users(&pool).await,
        vec![(1, "michael oxmaul".to_string())]
    );
}

#[tokio::test]
async fn missing_layer() {
    let app = axum::Router::new().route("/", axum::routing::get(|_: Tx| async move {}));
    let response = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_server_error());

    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    assert_eq!(
        body,
        format!("{}", axum_sea_orm_tx::Error::MissingExtension)
    );
}

#[tokio::test]
async fn overlapping_extractors() {
    let (_, _, response) = build_app(|_: Tx, _: Tx| async move {}).await;

    assert!(response.status.is_server_error());
    assert_eq!(
        response.body,
        format!("{}", axum_sea_orm_tx::Error::OverlappingExtractors)
    );
}

#[tokio::test]
async fn extractor_error_override() {
    let (_, _, response) = build_app(|_: Tx, _: Tx<MyError>| async move {}).await;

    assert!(response.status.is_client_error());
    assert_eq!(response.body, "internal server error");
}

#[tokio::test]
async fn layer_error_override() {
    let db = NamedTempFile::new().unwrap();
    let pool = Database::connect(&format!("sqlite://{}", db.path().display()))
        .await
        .unwrap();

    pool.execute(Statement::from_string(
        pool.get_database_backend(),
        "CREATE TABLE IF NOT EXISTS users (id INT PRIMARY KEY);".to_string(),
    ))
    .await
    .unwrap();
    pool.execute(Statement::from_string(
        pool.get_database_backend(),
        r#"
        CREATE TABLE IF NOT EXISTS comments (
            id INT PRIMARY KEY,
            user_id INT,
            FOREIGN KEY (user_id) REFERENCES users(id) DEFERRABLE INITIALLY DEFERRED
        );"#
        .to_string(),
    ))
    .await
    .unwrap();

    let app = axum::Router::new()
        .route(
            "/",
            axum::routing::get(|tx: Tx| async move {
                tx.execute(Statement::from_string(
                    tx.get_database_backend(),
                    "INSERT INTO comments VALUES (random(), random())".to_string(),
                ))
                .await
                .unwrap();
            }),
        )
        .layer(axum_sea_orm_tx::Layer::new_with_error::<MyError>(
            pool.clone(),
        ));

    let response = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();

    assert!(status.is_client_error());
    assert_eq!(body, "internal server error");
}

async fn insert_user(tx: &mut Tx, id: i32, name: &str) -> (i32, String) {
    tx.query_one(Statement::from_sql_and_values(
        tx.get_database_backend(),
        r#"INSERT INTO users VALUES (?, ?) RETURNING id, name;"#,
        vec![
            Value::Int(Some(id)),
            Value::String(Some(Box::new(name.to_string()))),
        ],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get_many("", &["id".to_string(), "name".to_string()])
    .unwrap()
}

async fn get_users(pool: &DatabaseConnection) -> Vec<(i32, String)> {
    pool.query_all(Statement::from_string(
        pool.get_database_backend(),
        "SELECT * from users".to_string(),
    ))
    .await
    .unwrap()
    .into_iter()
    .map(|res| {
        (
            res.try_get("", "id").unwrap(),
            res.try_get("", "name").unwrap(),
        )
    })
    .collect()
}

struct Response {
    status: http::StatusCode,
    body: axum::body::Bytes,
}

async fn build_app<H, T>(handler: H) -> (NamedTempFile, DatabaseConnection, Response)
where
    H: axum::handler::Handler<T, axum::body::Body>,
    T: 'static,
{
    let db = NamedTempFile::new().unwrap();
    let pool = Database::connect(&format!("sqlite://{}", db.path().display()))
        .await
        .unwrap();

    pool.execute(Statement::from_string(
        pool.get_database_backend(),
        "CREATE TABLE IF NOT EXISTS users (id INT PRIMARY KEY, name TEXT);".to_string(),
    ))
    .await
    .unwrap();

    let app = axum::Router::new()
        .route("/", axum::routing::get(handler))
        .layer(axum_sea_orm_tx::Layer::new(pool.clone()));

    let response = app
        .oneshot(
            http::Request::builder()
                .uri("/")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();

    (db, pool, Response { status, body })
}

struct MyError(axum_sea_orm_tx::Error);

impl From<axum_sea_orm_tx::Error> for MyError {
    fn from(error: axum_sea_orm_tx::Error) -> Self {
        Self(error)
    }
}

impl IntoResponse for MyError {
    fn into_response(self) -> axum::response::Response {
        (http::StatusCode::IM_A_TEAPOT, "internal server error").into_response()
    }
}

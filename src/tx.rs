//! A request extension that enables the [`Tx`](crate::Tx) extractor.

use std::marker::PhantomData;

use axum_core::{
    extract::{FromRequest, RequestParts},
    response::IntoResponse,
};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, StreamTrait, TransactionTrait,
};

use crate::{
    slot::{Lease, Slot},
    Error,
};

/// An `axum` extractor for a database transaction.
///
/// `&mut Tx` implements [`sea_orm::ConnectionTrait`] so it can be used directly with [`sea_orm::ConnectionTrait::execute`]
/// (and [`sea_orm::ConnectionTrait::query_one`], the corresponding macros, etc.):
///
/// ```
/// use axum_sea_orm_tx::Tx;
/// use sea_orm::ConnectionTrait;
///
/// async fn handler(mut tx: Tx) -> Result<(), sea_orm::DbErr> {
///     tx.execute(sea_orm::Statement::from_string(tx.get_database_backend(), "...".to_string())).await?;
///     /* ... */
/// #   Ok(())
/// }
/// ```
///
/// It also implements `Deref<Target = `[`sea_orm::DatabaseTransaction`]`>` and `DerefMut`, so you can call
/// methods from `DatabaseTransaction` and its traits:
///
/// ```
/// use axum_sea_orm_tx::Tx;
/// use sea_orm::TransactionTrait;
///
/// async fn handler(tx: Tx) -> Result<(), sea_orm::DbErr> {
///     let inner = tx.begin().await?;
///     /* ... */
/// #   Ok(())
/// }
/// ```
///
/// The `E` generic parameter controls the error type returned when the extractor fails. This can be
/// used to configure the error response returned when the extractor fails:
///
/// ```
/// use axum::response::IntoResponse;
/// use axum_sea_orm_tx::Tx;
///
/// struct MyError(axum_sea_orm_tx::Error);
///
/// // The error type must implement From<axum_sea_orm_tx::Error>
/// impl From<axum_sea_orm_tx::Error> for MyError {
///     fn from(error: axum_sea_orm_tx::Error) -> Self {
///         Self(error)
///     }
/// }
///
/// // The error type must implement IntoResponse
/// impl IntoResponse for MyError {
///     fn into_response(self) -> axum::response::Response {
///         (http::StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
///     }
/// }
///
/// async fn handler(tx: Tx<MyError>) {
///     /* ... */
/// }
/// ```
#[derive(Debug)]
pub struct Tx<E = Error>(Lease<DatabaseTransaction>, PhantomData<E>);

impl<E> Tx<E> {
    /// Explicitly commit the transaction.
    ///
    /// By default, the transaction will be committed when a successful response is returned
    /// (specifically, when the [`Service`](crate::Service) middleware intercepts an HTTP `2XX`
    /// response). This method allows the transaction to be committed explicitly.
    ///
    /// **Note:** trying to use the `Tx` extractor again after calling `commit` will currently
    /// generate [`Error::OverlappingExtractors`] errors. This may change in future.
    pub async fn commit(self) -> Result<(), DbErr> {
        self.0.steal().commit().await
    }
}

impl<E> AsRef<DatabaseTransaction> for Tx<E> {
    fn as_ref(&self) -> &DatabaseTransaction {
        &self.0
    }
}

impl<E> AsMut<DatabaseTransaction> for Tx<E> {
    fn as_mut(&mut self) -> &mut DatabaseTransaction {
        &mut self.0
    }
}

impl<E> std::ops::Deref for Tx<E> {
    type Target = DatabaseTransaction;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<E> std::ops::DerefMut for Tx<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<E: Sync> ConnectionTrait for Tx<E> {
    fn get_database_backend(&self) -> sea_orm::DbBackend {
        self.0.get_database_backend()
    }

    fn execute<'life0, 'async_trait>(
        &'life0 self,
        stmt: sea_orm::Statement,
    ) -> core::pin::Pin<
        Box<
            dyn core::future::Future<Output = Result<sea_orm::ExecResult, DbErr>>
                + core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        self.0.execute(stmt)
    }

    fn query_one<'life0, 'async_trait>(
        &'life0 self,
        stmt: sea_orm::Statement,
    ) -> core::pin::Pin<
        Box<
            dyn core::future::Future<Output = Result<Option<sea_orm::QueryResult>, DbErr>>
                + core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        self.0.query_one(stmt)
    }

    fn query_all<'life0, 'async_trait>(
        &'life0 self,
        stmt: sea_orm::Statement,
    ) -> core::pin::Pin<
        Box<
            dyn core::future::Future<Output = Result<Vec<sea_orm::QueryResult>, DbErr>>
                + core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        self.0.query_all(stmt)
    }
}

impl<E: Send + Sync> StreamTrait for Tx<E> {
    type Stream<'a> = <DatabaseTransaction as StreamTrait>::Stream<'a> where E: 'a;

    fn stream<'a>(
        &'a self,
        stmt: sea_orm::Statement,
    ) -> std::pin::Pin<
        Box<dyn futures_core::Future<Output = Result<Self::Stream<'a>, DbErr>> + 'a + Send>,
    > {
        self.0.stream(stmt)
    }
}

impl<E> TransactionTrait for Tx<E> {
    fn begin<'life0, 'async_trait>(
        &'life0 self,
    ) -> core::pin::Pin<
        Box<
            dyn core::future::Future<Output = Result<DatabaseTransaction, DbErr>>
                + core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        self.0.begin()
    }

    fn transaction<'life0, 'async_trait, F, T, TE>(
        &'life0 self,
        callback: F,
    ) -> core::pin::Pin<
        Box<
            dyn core::future::Future<Output = Result<T, sea_orm::TransactionError<TE>>>
                + core::marker::Send
                + 'async_trait,
        >,
    >
    where
        F: for<'c> FnOnce(
                &'c DatabaseTransaction,
            ) -> std::pin::Pin<
                Box<dyn futures_core::Future<Output = Result<T, TE>> + Send + 'c>,
            > + Send,
        T: Send,
        TE: std::error::Error + Send,
        F: 'async_trait,
        T: 'async_trait,
        TE: 'async_trait,
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        self.0.transaction(callback)
    }
}

impl<B, E> FromRequest<B> for Tx<E>
where
    B: Send,
    E: From<Error> + IntoResponse,
{
    type Rejection = E;

    fn from_request<'req, 'ctx>(
        req: &'req mut RequestParts<B>,
    ) -> futures_core::future::BoxFuture<'ctx, Result<Self, Self::Rejection>>
    where
        'req: 'ctx,
        Self: 'ctx,
    {
        Box::pin(async move {
            let ext: &mut Lazy = req
                .extensions_mut()
                .get_mut()
                .ok_or(Error::MissingExtension)?;

            let tx = ext.get_or_begin().await?;

            Ok(Self(tx, PhantomData))
        })
    }
}

/// The OG `Slot` – the transaction (if any) returns here when the `Extension` is dropped.
pub(crate) struct TxSlot(Slot<Option<Slot<DatabaseTransaction>>>);

impl TxSlot {
    /// Create a `TxSlot` bound to the given request extensions.
    ///
    /// When the request extensions are dropped, `commit` can be called to commit the transaction
    /// (if any).
    pub(crate) fn bind(extensions: &mut http::Extensions, pool: DatabaseConnection) -> Self {
        let (slot, tx) = Slot::new_leased(None);
        extensions.insert(Lazy { pool, tx });
        Self(slot)
    }

    pub(crate) async fn commit(self) -> Result<(), DbErr> {
        if let Some(tx) = self.0.into_inner().flatten().and_then(Slot::into_inner) {
            tx.commit().await?;
        }
        Ok(())
    }
}

/// A lazily acquired transaction.
///
/// When the transaction is started, it's inserted into the `Option` leased from the `TxSlot`, so
/// that when `Lazy` is dropped the transaction is moved to the `TxSlot`.
struct Lazy {
    pool: DatabaseConnection,
    tx: Lease<Option<Slot<DatabaseTransaction>>>,
}

impl Lazy {
    async fn get_or_begin(&mut self) -> Result<Lease<DatabaseTransaction>, Error> {
        let tx = if let Some(tx) = self.tx.as_mut() {
            tx
        } else {
            let tx = self.pool.begin().await?;
            self.tx.insert(Slot::new(tx))
        };

        tx.lease().ok_or(Error::OverlappingExtractors)
    }
}

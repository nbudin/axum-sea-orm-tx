//! A [`tower_layer::Layer`] that enables the [`Tx`](crate::Tx) extractor.

use std::marker::PhantomData;

use axum_core::response::IntoResponse;
use bytes::Bytes;
use futures_core::future::BoxFuture;
use http_body::{combinators::UnsyncBoxBody, Body};
use sea_orm::{DatabaseConnection, TransactionTrait};

use crate::{tx::TxSlot, Error};

/// A [`tower_layer::Layer`] that enables the [`Tx`] extractor.
///
/// This layer adds a lazily-initialised transaction to the [request extensions]. The first time the
/// [`Tx`] extractor is used on a request, a connection is acquired from the configured
/// [`sea_orm::DatabaseConnection`] and a transaction is started on it. The same transaction will be returned for
/// subsequent uses of [`Tx`] on the same request. The inner service is then called as normal. Once
/// the inner service responds, the transaction is committed or rolled back depending on the status
/// code of the response.
///
/// [`Tx`]: crate::Tx
/// [request extensions]: https://docs.rs/http/latest/http/struct.Extensions.html
pub struct Layer<C: TransactionTrait = DatabaseConnection, E = Error> {
    pool: C,
    _error: PhantomData<E>,
}

impl<C: TransactionTrait> Layer<C> {
    /// Construct a new layer with the given `pool`.
    ///
    /// A connection will be obtained from the pool the first time a [`Tx`](crate::Tx) is extracted
    /// from a request.
    ///
    /// If you want to access the pool outside of a transaction, you should add it also with
    /// [`axum::Extension`].
    ///
    /// To use a different type than [`Error`] to convert commit errors into responses, see
    /// [`new_with_error`](Self::new_with_error).
    ///
    /// [`axum::Extension`]: https://docs.rs/axum/latest/axum/extract/struct.Extension.html
    pub fn new(pool: C) -> Self {
        Self::new_with_error(pool)
    }

    /// Construct a new layer with a specific error type.
    ///
    /// See [`Layer::new`] for more information.
    pub fn new_with_error<E>(pool: C) -> Layer<C, E> {
        Layer {
            pool,
            _error: PhantomData,
        }
    }
}

impl<S, C: TransactionTrait + Clone, E> tower_layer::Layer<S> for Layer<C, E> {
    type Service = Service<S, C, E>;

    fn layer(&self, inner: S) -> Self::Service {
        Service {
            pool: self.pool.clone(),
            inner,
            _error: self._error,
        }
    }
}

/// A [`tower_service::Service`] that enables the [`Tx`](crate::Tx) extractor.
///
/// See [`Layer`] for more information.
pub struct Service<S, C: TransactionTrait = DatabaseConnection, E = Error> {
    pool: C,
    inner: S,
    _error: PhantomData<E>,
}

// can't simply derive because `DB` isn't `Clone`
impl<S: Clone, C: TransactionTrait + Clone, E> Clone for Service<S, C, E> {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            inner: self.inner.clone(),
            _error: self._error,
        }
    }
}

impl<S, C: TransactionTrait + Clone + Send + Sync + 'static, E, ReqBody, ResBody>
    tower_service::Service<http::Request<ReqBody>> for Service<S, C, E>
where
    S: tower_service::Service<
        http::Request<ReqBody>,
        Response = http::Response<ResBody>,
        Error = std::convert::Infallible,
    >,
    S::Future: Send + 'static,
    E: From<Error> + IntoResponse,
    ResBody: Body<Data = Bytes> + Send + 'static,
    ResBody::Error: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
{
    type Response = http::Response<UnsyncBoxBody<ResBody::Data, axum_core::Error>>;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(|err| match err {})
    }

    fn call(&mut self, mut req: http::Request<ReqBody>) -> Self::Future {
        let transaction = TxSlot::bind(req.extensions_mut(), self.pool.clone());

        let res = self.inner.call(req);

        Box::pin(async move {
            let res = res.await.unwrap(); // inner service is infallible

            if res.status().is_success() {
                if let Err(error) = transaction.commit().await {
                    return Ok(E::from(Error::Database { error }).into_response());
                }
            }

            Ok(res.map(|body| body.map_err(axum_core::Error::new).boxed_unsync()))
        })
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::DatabaseConnection;

    use super::Layer;

    // The trait shenanigans required by axum for layers are significant, so this "test" ensures
    // we've got it right.
    #[allow(unused, unreachable_code, clippy::diverging_sub_expression)]
    fn layer_compiles() {
        let pool: DatabaseConnection = todo!();

        let app = axum::Router::new()
            .route("/", axum::routing::get(|| async { "hello" }))
            .layer(Layer::new(pool));

        axum::Server::bind(todo!()).serve(app.into_make_service());
    }
}

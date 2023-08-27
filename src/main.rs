use axum::error_handling::HandleErrorLayer;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use http::StatusCode;
use std::borrow::Cow;
use std::fmt::{Display, Formatter};
use std::task::{Context, Poll};
use std::time::Duration;
use tower::{Layer, Service, ServiceBuilder};

#[tokio::main]
async fn main() {
    let my_layer = MyLayer {};

    let app = Router::new().route("/", get(hello))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_error))
                .layer(my_layer),
        );

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

// This works fine
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
// This doesn't, even though it's passing the same instance into the Box, and it's the same amount of implementing Error, Send, and Sync.
// pub type BoxError = Box<MyError>;

impl Display for MyError {
    fn fmt(&self, _: &mut Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for MyError {}

#[derive(Clone, Debug)]
pub struct MyError {}

async fn handle_error(error: BoxError) -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Cow::from(format!("Unhandled internal error: {}", error)),
    )
}

async fn hello() -> String {
    "Hello".to_owned()
}

// This Layer and Service were strongly based on tower::timeout, just with a different Error type.

#[derive(Clone, Debug)]
pub(crate) struct MyLayer {}

impl<S> Layer<S> for MyLayer {
    type Service = MyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MyService { inner }
    }
}

#[derive(Clone, Debug)]
pub struct MyService<S> {
    inner: S,
}

unsafe impl<S> Send for MyService<S> {}

impl<S, Request> Service<Request> for MyService<S>
where
    S: Service<Request> + Clone + Send + 'static,
    S::Error: Into<BoxError> + Send,
    S::Future: Send,
    S::Response: IntoResponse + Send,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = future::ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.inner.poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(r) => Poll::Ready(r.map_err(Into::into)),
        }
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let response = self.inner.call(request);
        let sleep = tokio::time::sleep(Duration::from_secs(15));

        future::ResponseFuture::new(response, sleep)
    }
}

mod future {
    use super::{BoxError, MyError};
    use pin_project_lite::pin_project;
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::time::Sleep;

    pin_project! {
        #[derive(Debug)]
        pub struct ResponseFuture<T> {
            #[pin]
            response: T,
            #[pin]
            sleep: Sleep,
        }
    }

    impl<T> ResponseFuture<T> {
        pub(crate) fn new(response: T, sleep: Sleep) -> Self {
            ResponseFuture { response, sleep }
        }
    }

    impl<F, T, E> Future for ResponseFuture<F>
    where
        F: Future<Output = Result<T, E>>,
        E: Into<BoxError>,
    {
        type Output = Result<T, BoxError>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();

            // First, try polling the future
            match this.response.poll(cx) {
                Poll::Ready(v) => return Poll::Ready(v.map_err(Into::into)),
                Poll::Pending => {}
            }

            // Now check the sleep
            match this.sleep.poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(_) => Poll::Ready(Err(Box::new(MyError {}))),
            }
        }
    }
}

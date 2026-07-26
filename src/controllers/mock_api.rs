//! Test-only helper: a [`kube::Client`] backed by a [`tower_test`] mock so
//! controller logic can be exercised against scripted apiserver responses.
//!
//! Only built under `#[cfg(test)]`. Production code never touches this. The
//! mock is a `tower::Service<http::Request<Body>>`; `ClientBuilder::new`
//! accepts any such service, so we get a real `Api` whose `get`/`create`/
//! `patch_status` calls flow through a channel we drive from the test.

#![cfg(test)]

use axum::http::{Request, Response, StatusCode};
use kube::Client;
use kube::client::{Body, ClientBuilder};
use tower_test::mock;

/// Handle used to script mock apiserver request/response pairs.
pub type MockHandle = mock::Handle<Request<Body>, Response<Body>>;

/// Build a [`kube::Client`] whose HTTP calls are served by a mock apiserver,
/// returning the [`MockHandle`] used to script responses.
pub fn mock_client() -> (Client, MockHandle) {
    let (svc, handle) = mock::pair::<Request<Body>, Response<Body>>();
    let client = ClientBuilder::new(svc, "default").build();
    (client, handle)
}

/// A Kubernetes `Status` response. kube maps a non-2xx body to
/// `Error::Api(Status { code, reason, .. })`, which the controllers match on to
/// detect 404 (NotFound) and 409 (AlreadyExists).
fn status_response(code: StatusCode, reason: &str) -> Response<Body> {
    let body = format!(
        r#"{{"kind":"Status","apiVersion":"v1","metadata":{{}},"status":"Failure","reason":"{reason}","code":{c}}}"#,
        c = code.as_u16()
    );
    Response::builder()
        .status(code)
        .body(Body::from(body.into_bytes()))
        .expect("static Status response builds")
}

/// A 404 NotFound apiserver response.
pub fn not_found() -> Response<Body> {
    status_response(StatusCode::NOT_FOUND, "NotFound")
}

/// A 409 AlreadyExists apiserver response.
pub fn already_exists() -> Response<Body> {
    status_response(StatusCode::CONFLICT, "AlreadyExists")
}

/// A 2xx response carrying a serialised object body.
fn object_response<T: serde::Serialize>(code: StatusCode, obj: &T) -> Response<Body> {
    let body = serde_json::to_vec(obj).expect("test object serialises");
    Response::builder()
        .status(code)
        .body(Body::from(body))
        .expect("static object response builds")
}

/// A 200 OK response carrying a serialised object body.
pub fn ok_object<T: serde::Serialize>(obj: &T) -> Response<Body> {
    object_response(StatusCode::OK, obj)
}

/// A 201 Created response carrying a serialised object body.
pub fn created_object<T: serde::Serialize>(obj: &T) -> Response<Body> {
    object_response(StatusCode::CREATED, obj)
}

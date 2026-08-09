//! A live server over a throwaway data directory.
//!
//! Compiled separately into every test binary, so anything one of them does not
//! use looks dead from inside that binary. The allow is about that, not about
//! genuinely unused code.
#![allow(dead_code)]

use std::sync::Arc;

use pane_core::{Core, CoreConfig};
use pane_serve::{Bound, ServeConfig, ServeHandle};

pub const TOKEN: &str = "test-token";

pub struct Harness {
    pub url: String,
    pub core: Arc<Core>,
    _serve: ServeHandle,
    _dir: tempfile::TempDir,
}

impl Harness {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        // `bootstrap`, not `attach_unowned`: a fresh directory has no database
        // to attach to. No instance lock, so these run in parallel with each
        // other and with a real Pane on the developer's machine.
        let core = Arc::new(
            Core::bootstrap(CoreConfig {
                data_dir: Some(dir.path().to_path_buf()),
                take_instance_lock: false,
            })
            .expect("core"),
        );

        // Port 0: the OS picks, so tests never collide on a fixed port.
        let bound: Bound = pane_serve::bind(ServeConfig {
            port: 0,
            token: Some(TOKEN.to_string()),
        })
        .await
        .expect("bind");

        let url = bound.url.clone();
        let serve = bound.serve(core.clone());

        Self {
            url,
            core,
            _serve: serve,
            _dir: dir,
        }
    }

    /// A client that does *not* follow redirects, so the token→cookie hop is
    /// observable rather than transparently followed.
    pub fn raw_client() -> reqwest::Client {
        Self::builder().build().expect("client")
    }

    /// A client already carrying the token, for tests about something else.
    pub fn authed() -> reqwest::Client {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().expect("header"),
        );
        Self::builder()
            .default_headers(headers)
            .build()
            .expect("client")
    }

    /// `no_proxy` is not optional here. reqwest honours `HTTP_PROXY` from the
    /// environment, and a developer debugging HTTPS traffic — the entire
    /// audience for this project — is likely to have one set. Without this the
    /// tests send loopback requests to an external proxy and get its errors
    /// back instead of the server's, which looks exactly like a broken handler.
    fn builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
    }

    pub fn at(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    /// POST /rpc, returning the status and the parsed body.
    pub async fn rpc(
        &self,
        op: &str,
        params: serde_json::Value,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        let res = Self::authed()
            .post(self.at("/rpc"))
            .json(&serde_json::json!({ "op": op, "params": params }))
            .send()
            .await
            .expect("rpc");
        let status = res.status();
        let body = res.json().await.unwrap_or(serde_json::Value::Null);
        (status, body)
    }
}

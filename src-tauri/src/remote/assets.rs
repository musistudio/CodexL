use super::{json_response, HttpBody};
use bytes::Bytes;
use http_body_util::Full;
use hyper::header::CONTENT_TYPE;
use hyper::{Response, StatusCode};
use serde_json::json;

const INDEX_HTML: &str = include_str!("../../../remote/control-pwa/index.html");
const CONTROL_HTML: &str = include_str!("../../../remote/control-pwa/control.html");
const APP_JS: &str = include_str!("../../../remote/control-pwa/app.js");
const REACT_APP_JS: &str = include_str!("../../../remote/control-pwa/react-app.js");
const QR_DECODER_JS: &str = include_str!("../../../remote/control-pwa/qrDecoder.js");
const REALTIME_TRANSPORT_JS: &str =
    include_str!("../../../remote/control-pwa/realtimeTransport.js");
const STYLES_CSS: &str = include_str!("../../../remote/control-pwa/styles.css");
const REACT_APP_CSS: &str = include_str!("../../../remote/control-pwa/react-app.css");
const MANIFEST: &str = include_str!("../../../remote/control-pwa/manifest.webmanifest");
const SERVICE_WORKER: &str = include_str!("../../../remote/control-pwa/service-worker.js");
const JS_QR_JS: &str = include_str!("../../../remote/control-pwa/vendor/jsQR.js");
const ICON_PNG: &[u8] = include_bytes!("../../icons/icon.png");
const FAVICON_32_PNG: &[u8] = include_bytes!("../../../remote/control-pwa/favicon-32x32.png");
const APPLE_TOUCH_ICON_PNG: &[u8] =
    include_bytes!("../../../remote/control-pwa/apple-touch-icon.png");
const ICON_192_PNG: &[u8] = include_bytes!("../../../remote/control-pwa/icon-192.png");
const ICON_512_PNG: &[u8] = include_bytes!("../../../remote/control-pwa/icon-512.png");
const ICON_MASKABLE_512_PNG: &[u8] =
    include_bytes!("../../../remote/control-pwa/icon-maskable-512.png");

pub(super) fn static_response(path: &str) -> Result<Response<HttpBody>, String> {
    match path {
        "/" | "/index.html" => {
            text_response(StatusCode::OK, "text/html; charset=utf-8", INDEX_HTML)
        }
        "/control.html" => text_response(StatusCode::OK, "text/html; charset=utf-8", CONTROL_HTML),
        "/app.js" => text_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            APP_JS,
        ),
        "/react-app.js" => text_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            REACT_APP_JS,
        ),
        "/qrDecoder.js" => text_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            QR_DECODER_JS,
        ),
        "/realtimeTransport.js" => text_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            REALTIME_TRANSPORT_JS,
        ),
        "/icon.png" => binary_response(StatusCode::OK, "image/png", ICON_PNG),
        "/favicon-32x32.png" => binary_response(StatusCode::OK, "image/png", FAVICON_32_PNG),
        "/apple-touch-icon.png" => {
            binary_response(StatusCode::OK, "image/png", APPLE_TOUCH_ICON_PNG)
        }
        "/icon-192.png" => binary_response(StatusCode::OK, "image/png", ICON_192_PNG),
        "/icon-512.png" => binary_response(StatusCode::OK, "image/png", ICON_512_PNG),
        "/icon-maskable-512.png" => {
            binary_response(StatusCode::OK, "image/png", ICON_MASKABLE_512_PNG)
        }
        "/manifest.webmanifest" => text_response(
            StatusCode::OK,
            "application/manifest+json; charset=utf-8",
            MANIFEST,
        ),
        "/service-worker.js" => text_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            SERVICE_WORKER,
        ),
        "/styles.css" => text_response(StatusCode::OK, "text/css; charset=utf-8", STYLES_CSS),
        "/react-app.css" => text_response(StatusCode::OK, "text/css; charset=utf-8", REACT_APP_CSS),
        "/vendor/jsQR.js" => text_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            JS_QR_JS,
        ),
        _ => Ok(json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "not found" }),
        )),
    }
}

fn text_response(
    status: StatusCode,
    content_type: &'static str,
    text: &'static str,
) -> Result<Response<HttpBody>, String> {
    Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .header(CONTENT_TYPE, content_type)
        .body(Full::new(Bytes::from_static(text.as_bytes())))
        .map_err(|e| e.to_string())
}

fn binary_response(
    status: StatusCode,
    content_type: &'static str,
    bytes: &'static [u8],
) -> Result<Response<HttpBody>, String> {
    Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .header(CONTENT_TYPE, content_type)
        .body(Full::new(Bytes::from_static(bytes)))
        .map_err(|e| e.to_string())
}

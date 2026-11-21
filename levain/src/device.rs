//! Phone hardware as platform calls: haptics and, with the `geolocation`
//! feature, the device's location. They run off the UI thread (a phone's
//! plugins answer on it) and reply when done. On desktop they are errors.

use serde::Deserialize;
use serde_json::Value;

use crate::bridge;
use crate::platform::Request;

/// The device calls.
pub const CALLS: [&str; 7] = [
    "haptics.vibrate",
    "haptics.impact",
    "haptics.notification",
    "haptics.selection",
    "location.current",
    "location.permission",
    "location.request_permission",
];

pub fn handles(call: &str) -> bool {
    CALLS.contains(&call)
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Impact {
    Light,
    Medium,
    Heavy,
    Soft,
    Rigid,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Feedback {
    Success,
    Warning,
    Error,
}

/// What a device call is asked to do, read from its arguments.
#[derive(Debug, PartialEq)]
pub enum Device {
    Vibrate(u32),
    Impact(Impact),
    Notification(Feedback),
    Selection,
    Location { high_accuracy: bool },
    LocationPermission { request: bool },
}

/// Reads `request`: an error for arguments it cannot take.
pub fn read(request: &Request) -> Result<Device, String> {
    let field = |key: &str| request.args.get(key).cloned().unwrap_or(Value::Null);
    let parse = |key: &str, default: &str| match field(key) {
        Value::Null => Value::from(default),
        value => value,
    };
    Ok(match request.call.as_str() {
        "haptics.vibrate" => Device::Vibrate(match field("duration") {
            Value::Null => 300,
            value => serde_json::from_value(value).map_err(|e| format!("`duration`: {e}"))?,
        }),
        "haptics.impact" => Device::Impact(
            serde_json::from_value(parse("style", "medium"))
                .map_err(|e| format!("`style`: {e}"))?,
        ),
        "haptics.notification" => Device::Notification(
            serde_json::from_value(parse("kind", "success")).map_err(|e| format!("`kind`: {e}"))?,
        ),
        "haptics.selection" => Device::Selection,
        "location.current" => Device::Location {
            high_accuracy: field("high_accuracy").as_bool().unwrap_or(false),
        },
        "location.permission" => Device::LocationPermission { request: false },
        "location.request_permission" => Device::LocationPermission { request: true },
        other => return Err(format!("no platform call `{other}`")),
    })
}

/// Runs a device call on a thread of its own and sends its reply.
pub fn handle<R: tauri::Runtime>(request: Request, app: &tauri::AppHandle<R>) {
    let app = app.clone();
    let outcome = read(&request);
    let reply = move |outcome: Result<Value, String>| {
        bridge::emit(&crate::platform::reply_for(&request, outcome));
    };
    match outcome {
        Err(error) => reply(Err(error)),
        Ok(device) => {
            std::thread::spawn(move || reply(run(&app, device)));
        }
    }
}

#[cfg(mobile)]
fn run<R: tauri::Runtime>(app: &tauri::AppHandle<R>, device: Device) -> Result<Value, String> {
    use tauri::Manager;
    use tauri_plugin_haptics::{HapticsExt, ImpactFeedbackStyle, NotificationFeedbackType};
    let haptics = || {
        app.try_state::<tauri_plugin_haptics::Haptics<R>>()
            .map(|_| app.haptics())
            .ok_or_else(|| "no haptics plugin in this host".to_owned())
    };
    let done = |result: Result<(), tauri_plugin_haptics::Error>| {
        result.map(|()| Value::Null).map_err(|e| e.to_string())
    };
    match device {
        Device::Vibrate(ms) => done(haptics()?.vibrate(ms)),
        Device::Impact(style) => done(haptics()?.impact_feedback(match style {
            Impact::Light => ImpactFeedbackStyle::Light,
            Impact::Medium => ImpactFeedbackStyle::Medium,
            Impact::Heavy => ImpactFeedbackStyle::Heavy,
            Impact::Soft => ImpactFeedbackStyle::Soft,
            Impact::Rigid => ImpactFeedbackStyle::Rigid,
        })),
        Device::Notification(kind) => done(haptics()?.notification_feedback(match kind {
            Feedback::Success => NotificationFeedbackType::Success,
            Feedback::Warning => NotificationFeedbackType::Warning,
            Feedback::Error => NotificationFeedbackType::Error,
        })),
        Device::Selection => done(haptics()?.selection_feedback()),
        Device::Location { high_accuracy } => location(app, high_accuracy),
        Device::LocationPermission { request } => location_permission(app, request),
    }
}

#[cfg(desktop)]
fn run<R: tauri::Runtime>(_: &tauri::AppHandle<R>, device: Device) -> Result<Value, String> {
    Err(match device {
        Device::Location { .. } | Device::LocationPermission { .. } => {
            "not available on desktop".to_owned()
        }
        _ => "no haptics on desktop".to_owned(),
    })
}

#[cfg(all(mobile, feature = "geolocation"))]
fn location<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    high_accuracy: bool,
) -> Result<Value, String> {
    use tauri_plugin_geolocation::{GeolocationExt, PositionOptions};
    let position = app
        .geolocation()
        .get_current_position(Some(PositionOptions {
            enable_high_accuracy: high_accuracy,
            timeout: 10_000,
            maximum_age: 0,
        }))
        .map_err(|e| e.to_string())?;
    Ok(position_json(&position))
}

#[cfg(all(mobile, feature = "geolocation"))]
fn position_json(position: &tauri_plugin_geolocation::Position) -> Value {
    let c = &position.coords;
    serde_json::json!({
        "latitude": c.latitude,
        "longitude": c.longitude,
        "accuracy": c.accuracy,
        "altitude": c.altitude,
        "speed": c.speed,
        "heading": c.heading,
        "timestamp": position.timestamp,
    })
}

#[cfg(all(mobile, feature = "geolocation"))]
fn location_permission<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    request: bool,
) -> Result<Value, String> {
    use tauri_plugin_geolocation::GeolocationExt;
    let geolocation = app.geolocation();
    let status = if request {
        geolocation.request_permissions(None)
    } else {
        geolocation.check_permissions()
    }
    .map_err(|e| e.to_string())?;
    serde_json::to_value(status.location).map_err(|e| e.to_string())
}

#[cfg(all(mobile, not(feature = "geolocation")))]
fn location<R: tauri::Runtime>(_: &tauri::AppHandle<R>, _: bool) -> Result<Value, String> {
    Err(NO_GEOLOCATION.into())
}

#[cfg(all(mobile, not(feature = "geolocation")))]
fn location_permission<R: tauri::Runtime>(
    _: &tauri::AppHandle<R>,
    _: bool,
) -> Result<Value, String> {
    Err(NO_GEOLOCATION.into())
}

#[cfg(all(mobile, not(feature = "geolocation")))]
const NO_GEOLOCATION: &str = "this host was built without the `geolocation` feature";

/// The plugins the device calls need, registered on phones.
pub fn plugins<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_haptics::init());
    #[cfg(all(mobile, feature = "geolocation"))]
    let builder = builder.plugin(tauri_plugin_geolocation::init());
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(call: &str, args: Value) -> Request {
        Request {
            call: call.into(),
            reference: 1,
            args,
        }
    }

    #[test]
    fn device_calls_read_their_arguments_with_defaults() {
        assert_eq!(
            read(&request("haptics.vibrate", json!({}))),
            Ok(Device::Vibrate(300))
        );
        assert_eq!(
            read(&request("haptics.vibrate", json!({"duration": 50}))),
            Ok(Device::Vibrate(50))
        );
        assert_eq!(
            read(&request("haptics.impact", json!({"style": "heavy"}))),
            Ok(Device::Impact(Impact::Heavy))
        );
        assert_eq!(
            read(&request("haptics.notification", Value::Null)),
            Ok(Device::Notification(Feedback::Success))
        );
        assert_eq!(
            read(&request("location.current", json!({"high_accuracy": true}))),
            Ok(Device::Location {
                high_accuracy: true
            })
        );
        assert!(
            read(&request("haptics.impact", json!({"style": "loud"})))
                .unwrap_err()
                .starts_with("`style`: unknown variant `loud`")
        );
        assert!(read(&request("haptics.vibrate", json!({"duration": -1}))).is_err());
    }

    #[test]
    fn on_desktop_they_say_they_are_not_available() {
        let h = crate::bridge::testing::harness();
        let app = tauri::test::mock_app();
        handle(request("haptics.selection", json!({})), app.handle());
        handle(request("location.current", json!({})), app.handle());
        let mut events = Vec::new();
        for _ in 0..100 {
            events.extend(h.events());
            if events.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut errors: Vec<_> = events.iter().map(|e| e["error"].clone()).collect();
        errors.sort_by_key(|e| e.to_string());
        assert_eq!(
            errors,
            [
                json!("haptics.selection: no haptics on desktop"),
                json!("location.current: not available on desktop")
            ]
        );
    }
}

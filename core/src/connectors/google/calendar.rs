//! Google Calendar's tools, on the Calendar API (`www.googleapis.com/calendar/v3`).
//!
//! Times go in as the model writes them: RFC 3339 with an offset, a local
//! date and time without one (this computer's time zone), or a bare date
//! for an all-day event.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone};
use reqwest::Method;
use serde_json::{Value, json};

use super::{Api, Args, ToolDef, ToolResult, pretty, segment};

const BASE: &str = "https://www.googleapis.com/calendar/v3";
const DEFAULT_RESULTS: u64 = 25;
const MAX_RESULTS: u64 = 100;
const DEFAULT_DAYS: i64 = 7;
const MAX_DESCRIPTION: usize = 800;

pub(super) fn tools() -> Vec<ToolDef> {
    let calendar_id = json!({"type": "string", "description": "A calendar's id from list_calendars; default: the user's main calendar."});
    let event_id = json!({"type": "string", "description": "The event's id, from list_events."});
    let time = "RFC 3339 (2026-10-23T15:00:00+02:00), a local date and time (2026-10-23T15:00), or a date (2026-10-23) for an all-day event.";
    vec![
        ToolDef {
            name: "list_calendars",
            description: "List the user's calendars: id, name, whether it's the main one, and their time zone.",
            properties: json!({}),
            required: &[],
            read_only: true,
            destructive: false,
        },
        ToolDef {
            name: "list_events",
            description: "List the events of a calendar between two times, in order (recurring events as single occurrences). \
                          Default: the next 7 days of the main calendar.",
            properties: json!({
                "calendar_id": calendar_id,
                "time_min": {"type": "string", "description": "From (a date or a date and time). Default: now."},
                "time_max": {"type": "string", "description": "Until (a date or a date and time). Default: 7 days after time_min."},
                "query": {"type": "string", "description": "Free text to match in the events."},
                "max_results": {"type": "integer", "description": "1 to 100 (default 25)."}
            }),
            required: &[],
            read_only: true,
            destructive: false,
        },
        ToolDef {
            name: "get_event",
            description: "Read one event in full: time, place, description, attendees and their answers, video call link.",
            properties: json!({"event_id": event_id, "calendar_id": calendar_id}),
            required: &["event_id"],
            read_only: true,
            destructive: false,
        },
        ToolDef {
            name: "create_event",
            description: "Create an event. Attendees, when given, get Google's invitation by email.",
            properties: json!({
                "summary": {"type": "string", "description": "The title."},
                "start": {"type": "string", "description": time},
                "end": {"type": "string", "description": "Like start. Default: one hour after it (the next day for an all-day event)."},
                "description": {"type": "string"},
                "location": {"type": "string"},
                "attendees": {"type": "array", "items": {"type": "string"}, "description": "Email addresses to invite."},
                "calendar_id": calendar_id
            }),
            required: &["summary", "start"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "update_event",
            description: "Change an event: only the fields given change. Attendees are told by email.",
            properties: json!({
                "event_id": event_id,
                "summary": {"type": "string"},
                "start": {"type": "string", "description": time},
                "end": {"type": "string"},
                "description": {"type": "string"},
                "location": {"type": "string"},
                "calendar_id": calendar_id
            }),
            required: &["event_id"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "respond_to_event",
            description: "Answer an invitation as the user: accepted, declined or tentative.",
            properties: json!({
                "event_id": event_id,
                "response": {"type": "string", "enum": ["accepted", "declined", "tentative"]},
                "calendar_id": calendar_id
            }),
            required: &["event_id", "response"],
            read_only: false,
            destructive: false,
        },
        ToolDef {
            name: "delete_event",
            description: "Delete an event; attendees are told by email.",
            properties: json!({"event_id": event_id, "calendar_id": calendar_id}),
            required: &["event_id"],
            read_only: false,
            destructive: true,
        },
    ]
}

pub(super) async fn call(api: &Api, name: &str, args: &Args) -> ToolResult {
    let calendar = segment(args.str("calendar_id").unwrap_or("primary"));
    let event_url = |args: &Args| -> Result<String, String> { Ok(format!("{BASE}/calendars/{calendar}/events/{}", segment(args.need("event_id")?))) };
    match name {
        "list_calendars" => list_calendars(api).await,
        "list_events" => list_events(api, &calendar, args).await,
        "get_event" => {
            let event = api.get(&event_url(args)?, &[]).await.map_err(|e| e.describe())?;
            Ok(pretty(&describe(&event, true)))
        }
        "create_event" => create_event(api, &calendar, args).await,
        "update_event" => update_event(api, &event_url(args)?, args).await,
        "respond_to_event" => respond(api, &event_url(args)?, args.need("response")?).await,
        "delete_event" => {
            api.send(Method::DELETE, &event_url(args)?, &[("sendUpdates", "all".into())], None).await.map_err(|e| e.describe())?;
            Ok("Event deleted.".into())
        }
        other => Err(format!("Google Calendar has no tool {other:?}")),
    }
}

async fn list_calendars(api: &Api) -> ToolResult {
    let list = api.get(&format!("{BASE}/users/me/calendarList"), &[]).await.map_err(|e| e.describe())?;
    let calendars: Vec<Value> = list["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| json!({"id": c["id"], "name": c["summaryOverride"].as_str().or(c["summary"].as_str()), "primary": c["primary"].as_bool().unwrap_or(false), "time_zone": c["timeZone"], "access": c["accessRole"]}))
        .collect();
    Ok(pretty(&json!({"calendars": calendars})))
}

async fn list_events(api: &Api, calendar: &str, args: &Args) -> ToolResult {
    let min = match args.str("time_min") {
        Some(s) => bound(s)?,
        None => Local::now().fixed_offset(),
    };
    let max = match args.str("time_max") {
        Some(s) => bound(s)?,
        None => min + Duration::days(DEFAULT_DAYS),
    };
    let count = args.int("max_results").unwrap_or(DEFAULT_RESULTS).clamp(1, MAX_RESULTS);
    let mut query = vec![
        ("timeMin", min.to_rfc3339()),
        ("timeMax", max.to_rfc3339()),
        ("singleEvents", "true".to_string()),
        ("orderBy", "startTime".to_string()),
        ("maxResults", count.to_string()),
    ];
    if let Some(q) = args.str("query") {
        query.push(("q", q.to_string()));
    }
    let list = api.get(&format!("{BASE}/calendars/{calendar}/events"), &query).await.map_err(|e| e.describe())?;
    let events: Vec<Value> = list["items"].as_array().into_iter().flatten().map(|e| describe(e, false)).collect();
    Ok(pretty(&json!({"from": min.to_rfc3339(), "until": max.to_rfc3339(), "time_zone": list["timeZone"], "events": events})))
}

async fn create_event(api: &Api, calendar: &str, args: &Args) -> ToolResult {
    let start = when(args.need("start")?)?;
    let end = match args.str("end") {
        Some(s) => when(s)?,
        None => default_end(&start)?,
    };
    let mut event = json!({"summary": args.need("summary")?, "start": start, "end": end});
    if let Some(d) = args.str("description") {
        event["description"] = json!(d);
    }
    if let Some(l) = args.str("location") {
        event["location"] = json!(l);
    }
    let attendees = args.list("attendees");
    if !attendees.is_empty() {
        event["attendees"] = attendees.iter().map(|a| json!({"email": a})).collect();
    }
    let made = api
        .send(Method::POST, &format!("{BASE}/calendars/{calendar}/events"), &[("sendUpdates", "all".into())], Some(event))
        .await
        .map_err(|e| e.describe())?;
    Ok(format!("Created.\n{}", pretty(&describe(&made, false))))
}

async fn update_event(api: &Api, url: &str, args: &Args) -> ToolResult {
    let mut patch = json!({});
    for key in ["summary", "description", "location"] {
        if let Some(v) = args.str(key) {
            patch[key] = json!(v);
        }
    }
    if let Some(s) = args.str("start") {
        let start = when(s)?;
        // A new start without a new end keeps the event's length only if we
        // knew it; an hour is the safer guess than an end before the start.
        if args.str("end").is_none() {
            patch["end"] = default_end(&start)?;
        }
        patch["start"] = start;
    }
    if let Some(e) = args.str("end") {
        patch["end"] = when(e)?;
    }
    if patch.as_object().is_some_and(|o| o.is_empty()) {
        return Err("nothing to change: give summary, start, end, description or location".into());
    }
    let updated = api.send(Method::PATCH, url, &[("sendUpdates", "all".into())], Some(patch)).await.map_err(|e| e.describe())?;
    Ok(format!("Updated.\n{}", pretty(&describe(&updated, false))))
}

async fn respond(api: &Api, url: &str, response: &str) -> ToolResult {
    if !["accepted", "declined", "tentative"].contains(&response) {
        return Err("response is accepted, declined or tentative".into());
    }
    let event = api.get(url, &[]).await.map_err(|e| e.describe())?;
    let mut attendees = event["attendees"].as_array().cloned().unwrap_or_default();
    let me = attendees.iter_mut().find(|a| a["self"].as_bool() == Some(true)).ok_or("the user isn't among this event's attendees")?;
    me["responseStatus"] = json!(response);
    api.send(Method::PATCH, url, &[("sendUpdates", "all".into())], Some(json!({"attendees": attendees}))).await.map_err(|e| e.describe())?;
    Ok(format!("Answered {response}."))
}

/// What the model reads of an event; `full` adds the long parts.
fn describe(e: &Value, full: bool) -> Value {
    let time = |t: &Value| t["dateTime"].as_str().or(t["date"].as_str()).map(str::to_string);
    let mut out = json!({
        "event_id": e["id"],
        "summary": e["summary"],
        "start": time(&e["start"]),
        "end": time(&e["end"]),
        "all_day": e["start"]["date"].is_string(),
    });
    for (key, from) in [("location", "location"), ("status", "status"), ("recurring_event_id", "recurringEventId")] {
        if let Some(v) = e[from].as_str() {
            out[key] = json!(v);
        }
    }
    if let Some(organizer) = e["organizer"]["email"].as_str() {
        out["organizer"] = json!(organizer);
    }
    let attendees: Vec<Value> = e["attendees"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| json!({"email": a["email"], "response": a["responseStatus"], "self": a["self"].as_bool().unwrap_or(false)}))
        .collect();
    if !attendees.is_empty() {
        out["attendees"] = json!(attendees);
    }
    if let Some(link) = e["hangoutLink"].as_str() {
        out["video_call"] = json!(link);
    }
    if let Some(d) = e["description"].as_str() {
        out["description"] = json!(if full { d.to_string() } else { cut(d, MAX_DESCRIPTION) });
    }
    if full && let Some(link) = e["htmlLink"].as_str() {
        out["link"] = json!(link);
    }
    out
}

fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// An event's start or end as the API takes it.
fn when(s: &str) -> Result<Value, String> {
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(json!({"date": date.to_string()}));
    }
    Ok(json!({"dateTime": instant(s)?.to_rfc3339()}))
}

/// One hour after a timed start, the next day after an all-day one.
fn default_end(start: &Value) -> Result<Value, String> {
    if let Some(date) = start["date"].as_str() {
        let next = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|e| e.to_string())? + Duration::days(1);
        return Ok(json!({"date": next.to_string()}));
    }
    let begin = DateTime::parse_from_rfc3339(start["dateTime"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(json!({"dateTime": (begin + Duration::hours(1)).to_rfc3339()}))
}

/// A bound of a time range: a date is its local midnight.
fn bound(s: &str) -> Result<DateTime<chrono::FixedOffset>, String> {
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return local(date.and_hms_opt(0, 0, 0).expect("midnight exists"));
    }
    instant(s)
}

/// RFC 3339, or a local date and time without an offset.
fn instant(s: &str) -> Result<DateTime<chrono::FixedOffset>, String> {
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok(t);
    }
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, format) {
            return local(naive);
        }
    }
    Err(format!("{s:?} is not a date or time I can read; use 2026-10-23T15:00 or 2026-10-23"))
}

fn local(naive: NaiveDateTime) -> Result<DateTime<chrono::FixedOffset>, String> {
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.fixed_offset())
        .ok_or_else(|| format!("{naive} doesn't exist here (a clock change)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_as_the_model_writes_them() {
        assert_eq!(when("2026-10-23").unwrap(), json!({"date": "2026-10-23"}));
        assert_eq!(when("2026-10-23T15:00:00+02:00").unwrap(), json!({"dateTime": "2026-10-23T15:00:00+02:00"}));
        let local = when("2026-10-23T15:00").unwrap();
        let t = DateTime::parse_from_rfc3339(local["dateTime"].as_str().unwrap()).unwrap();
        assert_eq!(t.naive_local().to_string(), "2026-10-23 15:00:00");
        assert!(when("tomorrow").is_err());
    }

    #[test]
    fn default_ends() {
        assert_eq!(default_end(&json!({"date": "2026-12-31"})).unwrap(), json!({"date": "2027-01-01"}));
        assert_eq!(default_end(&json!({"dateTime": "2026-10-23T15:00:00+02:00"})).unwrap(), json!({"dateTime": "2026-10-23T16:00:00+02:00"}));
    }

    #[test]
    fn an_event_for_the_model() {
        let e = json!({"id": "e1", "summary": "Standup", "start": {"date": "2026-10-23"}, "end": {"date": "2026-10-24"},
            "attendees": [{"email": "me@x.it", "self": true, "responseStatus": "needsAction"}], "description": "x".repeat(900), "htmlLink": "https://cal"});
        let short = describe(&e, false);
        assert_eq!(short["all_day"], true);
        assert_eq!(short["attendees"][0]["response"], "needsAction");
        assert_eq!(short["description"].as_str().unwrap().chars().count(), MAX_DESCRIPTION + 1);
        assert!(short.get("link").is_none());
        assert_eq!(describe(&e, true)["link"], "https://cal");
    }
}

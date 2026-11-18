// The Mac's own calendar (EventKit), for the meeting detection
// (docs/meetings.md, Detection): every account added to Calendar.app
// (iCloud, Google, Exchange) is there, with no connector to set up.

import EventKit
import Foundation

private let iso: ISO8601DateFormatter = {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime]
    return f
}()

/// Full access to the events, asking the first time (the prompt names Yoru,
/// which started this helper).
private func calendarAccess(_ store: EKEventStore) -> Bool {
    switch EKEventStore.authorizationStatus(for: .event) {
    case .fullAccess:
        return true
    case .notDetermined:
        let done = DispatchSemaphore(value: 0)
        var granted = false
        store.requestFullAccessToEvents { ok, _ in
            granted = ok
            done.signal()
        }
        done.wait()
        return granted
    default:
        return false
    }
}

/// The events from `from` to `to` (Unix ms) as one JSON object:
/// `{"events": [...]}`, or `{"error": "denied"}` without access.
func listEvents(from: Double, to: Double) {
    let store = EKEventStore()
    guard calendarAccess(store) else {
        emit(["error": "denied"])
        return
    }
    let start = Date(timeIntervalSince1970: from / 1000)
    let end = Date(timeIntervalSince1970: to / 1000)
    let predicate = store.predicateForEvents(withStart: start, end: end, calendars: nil)
    let events: [[String: Any]] = store.events(matching: predicate).map { e in
        let attendees: [[String: Any]] = (e.attendees ?? []).map { a in
            ["self": a.isCurrentUser, "response": a.participantStatus == .declined ? "declined" : "other"]
        }
        return [
            "event_id": e.calendarItemIdentifier + "@" + iso.string(from: e.startDate),
            "summary": e.title ?? "",
            "start": iso.string(from: e.startDate),
            "end": iso.string(from: e.endDate),
            "all_day": e.isAllDay,
            "status": e.status == .canceled ? "cancelled" : "confirmed",
            "attendees": attendees,
            "location": e.location ?? "",
            "description": [e.url?.absoluteString ?? "", e.notes ?? ""].joined(separator: " "),
        ]
    }
    emit(["events": events])
}

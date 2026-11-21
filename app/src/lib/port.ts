/** A connection's port is on the WiFi: `wifi:<id>` as a target, `WiFi <ip>` once connected (core/src/bridge/session.rs). */
export const onWifi = (port: string | null | undefined): boolean => !!port && /^wifi[: ]/i.test(port);

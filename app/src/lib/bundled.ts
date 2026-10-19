// The firmware bundled with the app: its version and its changelog, read from
// the repository's firmware/yoru/ at build time (the image itself is embedded
// by the Rust side).

// Vite reads these options at build time: they must stay literals.
const versions = import.meta.glob<string>("../../../firmware/yoru/version.txt", { query: "?raw", import: "default", eager: true });
const changelogs = import.meta.glob<string>("../../../firmware/yoru/FIRMWARE_CHANGELOG.md", { query: "?raw", import: "default", eager: true });

/** The version the app flashes; undefined when firmware/yoru/ is empty. */
export const BUNDLED_VERSION: string | undefined = Object.values(versions)[0]?.trim();

/** Its FIRMWARE_CHANGELOG.md. */
export const BUNDLED_CHANGELOG: string = Object.values(changelogs)[0] ?? "";

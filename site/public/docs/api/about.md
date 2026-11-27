# About

Method and path: GET /api/v1/about.

Area: Status and output.

## Request

**GET /api/v1/about**

Reports the app version, build, Windows edition, service version and how the previous run ended.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/about"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/about", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `About`.

| Field | Type | What it holds |
| --- | --- | --- |
| `version` | `string` | The daemon's version (`CARGO_PKG_VERSION`), the same as `version` |
| `commit` | `string or null` | The short commit the daemon was built from, when the build knew it Optional. |
| `build_date` | `string or null` | The build date, `YYYY-MM-DD`, when the build knew it Optional. |
| `os` | `string or null` | Windows edition and build, for example `Windows 11 Home 10.0.26300`, when it could be read Optional. |
| `service_version` | `string or null` | The service's version from its `Hello`, when the service is connected Optional. |
| `last_exit` | `LastExit` | How the previous run ended |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/about` | none | About |
| GET `/api/v1/about/report` | none | AboutReport |
| GET `/api/v1/output` | none | OutputState |
| PATCH `/api/v1/output` | PatchOutput | OutputState |
| POST `/api/v1/quit` | none | JSON object |
| GET `/api/v1/status` | none | Status |

## Example

**GET**

Reports the app version, build, Windows edition, service version and how the previous run ended.

This route takes no request body.

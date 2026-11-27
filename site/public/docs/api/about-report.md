# About report

Method and path: GET /api/v1/about/report.

Area: Status and output.

## Request

**GET /api/v1/about/report**

Returns the scrubbed support report the editor shows and copies, with the fields the bug report template fills.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/about/report"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/about/report", {
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

**Answer body**: `AboutReport`.

| Field | Type | What it holds |
| --- | --- | --- |
| `markdown` | `string` | The whole report as GitHub Markdown: what the preview shows and Copy puts on the clipboard |
| `version` | `string` | The bug template's `version` field |
| `windows` | `string` | The bug template's `windows` field |
| `hardware` | `string` | The bug template's `hardware` field: one line per device, model-level only |
| `other_rgb_apps` | `string` | The bug template's `other-rgb-apps` field |

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

Returns the scrubbed support report the editor shows and copies, with the fields the bug report template fills.

This route takes no request body.

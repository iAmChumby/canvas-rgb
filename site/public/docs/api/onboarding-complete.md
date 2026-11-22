# Complete onboarding

Method and path: POST /api/v1/onboarding/complete.

Area: Settings and update.

## Request

**POST /api/v1/onboarding/complete**

Records that first-run onboarding has finished.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/onboarding/complete"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/onboarding/complete", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `JSON object`.

The fields the daemon sends: the flag was saved.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the flag was saved. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/config/export` | none | ExportBundle |
| POST `/api/v1/config/import` | ExportBundle | JSON object |
| POST `/api/v1/first-run/dismiss-zone-panel` | none | JSON object |
| POST `/api/v1/onboarding/complete` | none | JSON object |
| GET `/api/v1/settings` | none | Settings |
| PUT `/api/v1/settings` | Settings | Settings |
| POST `/api/v1/update/install` | InstallUpdateRequest | InstallUpdateResponse |

## Example

**POST**

Records that first-run onboarding has finished.

This route takes no request body.

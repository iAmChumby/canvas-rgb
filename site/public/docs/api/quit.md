# Quit

Method and path: POST /api/v1/quit.

Area: Status and output.

## Request

**POST /api/v1/quit**

Stops the daemon after applying its exit lighting setting and telling the fan and update service the stop was deliberate.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/quit"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/quit", {
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

The fields the daemon sends: the stop request was accepted.

| Field | Type | What it holds |
| --- | --- | --- |
| `quitting` | `boolean` | True when the stop request was accepted. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/output` | none | OutputState |
| PATCH `/api/v1/output` | PatchOutput | OutputState |
| POST `/api/v1/quit` | none | JSON object |
| GET `/api/v1/status` | none | Status |

## Example

**POST**

Stops the daemon after applying its exit lighting setting and telling the fan and update service the stop was deliberate.

This route takes no request body.

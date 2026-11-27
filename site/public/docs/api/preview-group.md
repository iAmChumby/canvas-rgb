# Group preview

Method and path: GET /api/v1/preview/group.

Area: Preview.

## Request

**GET /api/v1/preview/group**

Streams the picture one group draws at the group's brightness. Each binary frame holds a 16-bit little-endian width, a 16-bit little-endian height, then width times height RGB triples.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl "ws://127.0.0.1:9430/api/v1/preview/group?token=$TOKEN&group=everything"
# Needs a curl built with WebSocket support (7.86 or later); the JavaScript sample is the main example.
```

```js
const token = process.env.CANVAS_RGB_TOKEN;
const socket = new WebSocket("ws://127.0.0.1:9430/api/v1/preview/group?token=" + encodeURIComponent(token) + "&group=everything");
socket.binaryType = "arraybuffer";
socket.onmessage = (event) => {
  console.log(new Uint8Array(event.data));
};
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `WebSocket (binary frames)`.

No field list is published for this body.

| Query | Type | What it holds |
| --- | --- | --- |
| `token` | `string` | The API token, in the query string because a browser cannot set a header on a WebSocket handshake. |
| `group` | `string` | The id of a group in the active scene. The default scene's only group has the id everything. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/preview` | none | WebSocket (binary frames) |
| GET `/api/v1/preview/group` | none | WebSocket (binary frames) |
| GET `/api/v1/preview/lights` | none | WebSocket (binary frames) |

## Example

**GET**

Streams the picture one group draws at the group's brightness. Each binary frame holds a 16-bit little-endian width, a 16-bit little-endian height, then width times height RGB triples.

This route takes no request body.

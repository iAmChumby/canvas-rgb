# Header setup

Method and path: POST / DELETE /api/v1/chains/{device}/{zone}/setup.

Area: Scenes and chains.

## Request

**POST /api/v1/chains/{device}/{zone}/setup**

Starts a setup session for one header.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/chains/<device>/<zone>/setup"
# The answer carries the session id. The setup requests that follow send it in their body.
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/chains/<device>/<zone>/setup", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**DELETE /api/v1/chains/{device}/{zone}/setup**

Ends a header setup, restoring the earlier length and the scene.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"session":"setup-7f3a91c2"}' "http://127.0.0.1:9430/api/v1/chains/<device>/<zone>/setup"
# Send the session id the setup POST answered with.
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/chains/<device>/<zone>/setup", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"session":"setup-7f3a91c2"}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `SetupStarted`.

| Field | Type | What it holds |
| --- | --- | --- |
| `session` | `string` | The id the setup answered with. |
| `restore_to` | `number` | The length the header goes back to if the setup ends without a commit. |
| `raised_to` | `number` | The length the header was raised to: the zone's maximum. |

**DELETE**

**Request body**: `SetupEmpty`.

| Field | Type | What it holds |
| --- | --- | --- |
| `session` | `string` | The id the setup answered with. |

**Answer body**: `JSON object`.

The fields the daemon sends: the setup ended and the header's length after the restore.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the setup ended. |
| `leds` | `number` | The header's length after the restore. |

| Path | Type | What it holds |
| --- | --- | --- |
| `device` | `string` | The id of the device the route names. |
| `zone` | `number` | The index of the zone, starting at 0. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/chains` | none | ChainsResponse |
| POST `/api/v1/chains/{device}/{zone}/setup` | none | SetupStarted |
| DELETE `/api/v1/chains/{device}/{zone}/setup` | SetupEmpty | JSON object |
| POST `/api/v1/chains/{device}/{zone}/setup/commit` | SetupCommit | SetupCommitted |
| POST `/api/v1/chains/{device}/{zone}/setup/empty` | SetupEmpty | JSON object |
| POST `/api/v1/chains/{device}/{zone}/setup/light` | SetupLight | JSON object |
| GET `/api/v1/scenes` | none | ScenesResponse |
| POST `/api/v1/scenes` | SceneCreate | SceneAnswer |
| PUT `/api/v1/scenes` | ScenesWrite | ScenesResponse |
| PUT `/api/v1/scenes/{id}` | ScenePut | SceneAnswer |
| DELETE `/api/v1/scenes/{id}` | none | ScenesResponse |
| PUT `/api/v1/scenes/{id}/groups/{group}/brightness` | GroupBrightnessPut | Scene |
| POST `/api/v1/scenes/{id}/groups/{group}/move` | GroupMove | GroupMoved |
| PATCH `/api/v1/scenes/{id}/groups/{group}/props` | Props | Scene |
| PUT `/api/v1/scenes/active` | SceneActivePut | ScenesResponse |

## Example

**POST**

Starts a setup session for one header.

This route takes no request body.

**DELETE**

Ends a header setup, restoring the earlier length and the scene.

```json
{
  "session": "setup-7f3a91c2"
}
```

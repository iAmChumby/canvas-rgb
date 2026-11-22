# Setup commit

Method and path: POST /api/v1/chains/{device}/{zone}/setup/commit.

Area: Scenes and chains.

## Request

**POST /api/v1/chains/{device}/{zone}/setup/commit**

Applies the chosen zone sizes to a header and saves them.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"session":"setup-7f3a91c2","parts":[{"name":"Fan run","kind":"fan","leds":12}]}' "http://127.0.0.1:9430/api/v1/chains/<device>/<zone>/setup/commit"
# Send the session id the setup POST answered with.
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/chains/<device>/<zone>/setup/commit", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"session":"setup-7f3a91c2","parts":[{"name":"Fan run","kind":"fan","leds":12}]}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `SetupCommit`.

| Field | Type | What it holds |
| --- | --- | --- |
| `session` | `string` | The id the setup answered with. |
| `parts` | `array of SetupPart` | The parts in chain order; their LED counts add up to the final length. |

**Answer body**: `SetupCommitted`.

| Field | Type | What it holds |
| --- | --- | --- |
| `lights` | `array of LightView` | The lights the setup made, with what the editor shows for each. |
| `final_leds` | `number` | The header's final LED count. |

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

Applies the chosen zone sizes to a header and saves them.

```json
{
  "session": "setup-7f3a91c2",
  "parts": [
    {
      "name": "Fan run",
      "kind": "fan",
      "leds": 12
    }
  ]
}
```

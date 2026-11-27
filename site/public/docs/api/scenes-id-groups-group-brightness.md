# Group brightness

Method and path: PUT /api/v1/scenes/{id}/groups/{group}/brightness.

Area: Scenes and chains.

## Request

**PUT /api/v1/scenes/{id}/groups/{group}/brightness**

Sets the brightness of one group of a scene.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"brightness":80}' "http://127.0.0.1:9430/api/v1/scenes/<id>/groups/<group>/brightness"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/scenes/<id>/groups/<group>/brightness", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"brightness":80}),
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `GroupBrightnessPut`.

| Field | Type | What it holds |
| --- | --- | --- |
| `brightness` | `number` | The brightness of the group, from 0 to 100. |

**Answer body**: `Scene`.

| Field | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the scene. |
| `name` | `string` | The name of the scene. |
| `rev` | `number` | The revision of the scene; it moves with every saved change. |
| `groups` | `array of Group` | The groups the scene holds. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the scene that holds the group. |
| `group` | `string` | The id of a group in the scene named by the `id` path parameter. Any scene, active or not. |

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

**PUT**

Sets the brightness of one group of a scene.

```json
{
  "brightness": 80
}
```

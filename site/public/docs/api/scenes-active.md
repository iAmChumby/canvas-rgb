# Active scene

Method and path: PUT /api/v1/scenes/active.

Area: Scenes and chains.

## Request

**PUT /api/v1/scenes/active**

Makes a scene active, with a fade.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"id":"everything"}' "http://127.0.0.1:9430/api/v1/scenes/active"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/scenes/active", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"id":"everything"}),
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `SceneActivePut`.

| Field | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the scene to use. |

**Answer body**: `ScenesResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `active` | `string` | The id of the active scene. |
| `scenes` | `array of Scene` | The saved scenes. |
| `group_states` | `array of GroupState` | The active scene's groups, in its order. |

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

Makes a scene active, with a fade.

```json
{
  "id": "everything"
}
```

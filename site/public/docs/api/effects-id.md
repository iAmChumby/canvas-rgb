# Effect

Method and path: GET / PUT / DELETE /api/v1/effects/{id}.

Area: Effects.

## Request

**GET /api/v1/effects/{id}**

Returns one effect's details, including its properties and presets.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/effects/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PUT /api/v1/effects/{id}**

Saves an effect's property values, user presets and favourite; the effect must already exist.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"schema":{"id":"html:demo","name":"Demo","description":"A short demo.","needs":{"screen":false,"audio":false},"properties":[],"presets":[]},"favorite":true,"user_presets":[{"name":"Cold","props":{"speed":{"type":"number","value":30}}}],"props":{"speed":{"type":"number","value":30}}}' "http://127.0.0.1:9430/api/v1/effects/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"schema":{"id":"html:demo","name":"Demo","description":"A short demo.","needs":{"screen":false,"audio":false},"properties":[],"presets":[]},"favorite":true,"user_presets":[{"name":"Cold","props":{"speed":{"type":"number","value":30}}}],"props":{"speed":{"type":"number","value":30}}}),
});
const data = await response.json();
```

**DELETE /api/v1/effects/{id}**

Removes one user HTML effect from the library.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/effects/<id>?keep_presets=true"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>?keep_presets=true", {
  method: "DELETE",
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

**Answer body**: `EffectDetail`.

| Field | Type | What it holds |
| --- | --- | --- |
| `schema` | `PropertySchema` | The effect's property schema. |
| `favorite` | `boolean` | True marks the effect as a favourite. |
| `user_presets` | `array of Preset` | The user's saved presets for this effect. The built-in ones are in the schema. |
| `props` | `an object keyed by text, with PropValue values` | The values the effect runs with when it is active, or its saved values, or the schema's defaults. |

**PUT**

**Request body**: `EffectDetail`.

| Field | Type | What it holds |
| --- | --- | --- |
| `schema` | `PropertySchema` | The effect's property schema. |
| `favorite` | `boolean` | True marks the effect as a favourite. |
| `user_presets` | `array of Preset` | The user's saved presets for this effect. The built-in ones are in the schema. |
| `props` | `an object keyed by text, with PropValue values` | The values the effect runs with when it is active, or its saved values, or the schema's defaults. |

**Answer body**: `JSON object`.

The fields the daemon sends: the effect whose values were saved.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the values were saved. |
| `id` | `string` | The effect whose values were saved. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `DeleteEffectResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the effect was deleted. |
| `id` | `string` | The id of the effect that was deleted. |
| `presets_removed` | `number` | How many presets went with the effect. |
| `draft_ended` | `DraftEnded` | Set when the delete ended a draft on the lights, so the editor can say the lights fell back. A delete with no draft on the lights leaves it out. Optional. |

| Query | Type | What it holds |
| --- | --- | --- |
| `keep_presets` | `boolean` | True keeps a removed effect's saved presets in the library, so importing the file again brings them back. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the effect the route names. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/effects` | none | EffectsResponse |
| GET `/api/v1/effects/{id}` | none | EffectDetail |
| PUT `/api/v1/effects/{id}` | EffectDetail | JSON object |
| DELETE `/api/v1/effects/{id}` | none | DeleteEffectResponse |
| POST `/api/v1/effects/{id}/activate` | ActivateEffect | JSON object |
| PATCH `/api/v1/effects/{id}/defaults` | Props | JSON object |
| PUT `/api/v1/effects/{id}/favorite` | SetFavorite | JSON object |
| POST `/api/v1/effects/{id}/presets` | PresetCreate | PresetsResponse |
| PUT `/api/v1/effects/{id}/presets/{name}` | PresetReplace | PresetsResponse |
| DELETE `/api/v1/effects/{id}/presets/{name}` | none | PresetsResponse |
| GET `/api/v1/effects/{id}/source` | none | EffectSource |
| GET `/api/v1/effects/{id}/thumbnail` | none | Binary thumbnail |
| GET `/api/v1/effects/draft` | none | DraftStatus |
| PUT `/api/v1/effects/draft` | DraftRequest | DraftStatus |
| DELETE `/api/v1/effects/draft` | none | DraftStatus |
| POST `/api/v1/effects/import` | ImportEffect | ImportEffectResponse |
| POST `/api/v1/effects/preview` | EffectPreviewRequest | EffectPreviewResponse |

## Example

**GET**

Returns one effect's details, including its properties and presets.

This route takes no request body.

**PUT**

Saves an effect's property values, user presets and favourite; the effect must already exist.

```json
{
  "schema": {
    "id": "html:demo",
    "name": "Demo",
    "description": "A short demo.",
    "needs": {
      "screen": false,
      "audio": false
    },
    "properties": [],
    "presets": []
  },
  "favorite": true,
  "user_presets": [
    {
      "name": "Cold",
      "props": {
        "speed": {
          "type": "number",
          "value": 30
        }
      }
    }
  ],
  "props": {
    "speed": {
      "type": "number",
      "value": 30
    }
  }
}
```

**DELETE**

Removes one user HTML effect from the library.

This route takes no request body.

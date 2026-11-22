# Effect presets

Method and path: POST /api/v1/effects/{id}/presets.

Area: Effects.

## Request

**POST /api/v1/effects/{id}/presets**

Saves a named preset of an effect's property values.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"name":"Warm","props":{"speed":{"type":"number","value":30}}}' "http://127.0.0.1:9430/api/v1/effects/<id>/presets"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>/presets", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"name":"Warm","props":{"speed":{"type":"number","value":30}}}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `PresetCreate`.

| Field | Type | What it holds |
| --- | --- | --- |
| `name` | `string` | The name of the preset. |
| `props` | `an object keyed by text, with PropValue values` | The property values the preset saves, by property key. |

**Answer body**: `PresetsResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `user_presets` | `array of Preset` | The user's saved presets. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the effect the preset belongs to. |

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

**POST**

Saves a named preset of an effect's property values.

```json
{
  "name": "Warm",
  "props": {
    "speed": {
      "type": "number",
      "value": 30
    }
  }
}
```

# Config import

Method and path: POST /api/v1/config/import.

Area: Settings and update.

## Request

**POST /api/v1/config/import**

Validates and imports a configuration bundle. The bundle replaces every saved setting, scene, light, fan, display, stack and pattern with the one it carries, so export the current configuration first.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"schema_version":1,"exported_at":"2026-11-21T12:00:00Z","app_version":"0.18.1","settings":{"fps":30,"black_on_lock":false,"on_quit":"black","boot_color":"#1b2c3d","brightness":80,"power":"running","warmth":6500},"layouts":{"schema_version":1,"active":"","layouts":[]},"zones":{"schema_version":1,"sizes":{}},"effects":{"schema_version":1,"saved":{},"presets":{},"favorites":[]},"fans":{"schema_version":1,"floor_percent":20,"active_profile":"Default","profiles":[{"name":"Default","headers":{}}],"boot_color":"#1b2c3d","hot_limits":{"cpu_c":null,"gpu_c":null,"board_c":null}},"displays":[{"device":"display:5f6a:7b8c","source":{"kind":"api"},"unit":"celsius","decimals":0,"blink_above":null}],"stacks":[{"id":"stack:example","layers":[]},{"id":"stack:other","layers":[]}],"patterns":[{"id":"pattern:Example","built_on":{"led_count":4,"arrangement":"ring"},"frames":[{"colors":["#d21e28","#0f9d58","#1a73e8","#fbbc04"],"hold":0}],"motion":{"type":"none"}},{"id":"pattern:Other","built_on":{"led_count":4,"arrangement":"ring"},"frames":[{"colors":["#101010","#202020","#303030","#404040"],"hold":0}],"motion":{"type":"none"}}],"lights":{"schema_version":1,"chains":[],"lights":[{"id":"openrgb:2a3b4c5d#z0","device_id":"openrgb:2a3b4c5d","zone_index":0,"arrangement":"strip","reversed":false},{"id":"openrgb:6e7f8091#z0","device_id":"openrgb:6e7f8091","zone_index":0,"arrangement":"strip","reversed":false},{"id":"openrgb:HID:VID_3151&PID_4015#z0","device_id":"openrgb:HID:VID_3151&PID_4015","zone_index":0,"arrangement":"strip","reversed":false,"name":"Old keyboard lights"}]},"scenes":{"schema_version":1,"active":"everything","scenes":[{"id":"everything","name":"Everything","rev":1,"groups":[{"id":"everything","name":"Everything","members":["openrgb:2a3b4c5d#z0"],"plays":{"type":"effect","effect":"solid","props":{}},"mode":"flow_across","brightness":100}]},{"id":"scene:other","name":"Other","rev":1,"groups":[]},{"id":"scene:retired","name":"Retired","rev":1,"groups":[]}]}}' "http://127.0.0.1:9430/api/v1/config/import"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/config/import", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"schema_version":1,"exported_at":"2026-11-21T12:00:00Z","app_version":"0.18.1","settings":{"fps":30,"black_on_lock":false,"on_quit":"black","boot_color":"#1b2c3d","brightness":80,"power":"running","warmth":6500},"layouts":{"schema_version":1,"active":"","layouts":[]},"zones":{"schema_version":1,"sizes":{}},"effects":{"schema_version":1,"saved":{},"presets":{},"favorites":[]},"fans":{"schema_version":1,"floor_percent":20,"active_profile":"Default","profiles":[{"name":"Default","headers":{}}],"boot_color":"#1b2c3d","hot_limits":{"cpu_c":null,"gpu_c":null,"board_c":null}},"displays":[{"device":"display:5f6a:7b8c","source":{"kind":"api"},"unit":"celsius","decimals":0,"blink_above":null}],"stacks":[{"id":"stack:example","layers":[]},{"id":"stack:other","layers":[]}],"patterns":[{"id":"pattern:Example","built_on":{"led_count":4,"arrangement":"ring"},"frames":[{"colors":["#d21e28","#0f9d58","#1a73e8","#fbbc04"],"hold":0}],"motion":{"type":"none"}},{"id":"pattern:Other","built_on":{"led_count":4,"arrangement":"ring"},"frames":[{"colors":["#101010","#202020","#303030","#404040"],"hold":0}],"motion":{"type":"none"}}],"lights":{"schema_version":1,"chains":[],"lights":[{"id":"openrgb:2a3b4c5d#z0","device_id":"openrgb:2a3b4c5d","zone_index":0,"arrangement":"strip","reversed":false},{"id":"openrgb:6e7f8091#z0","device_id":"openrgb:6e7f8091","zone_index":0,"arrangement":"strip","reversed":false},{"id":"openrgb:HID:VID_3151&PID_4015#z0","device_id":"openrgb:HID:VID_3151&PID_4015","zone_index":0,"arrangement":"strip","reversed":false,"name":"Old keyboard lights"}]},"scenes":{"schema_version":1,"active":"everything","scenes":[{"id":"everything","name":"Everything","rev":1,"groups":[{"id":"everything","name":"Everything","members":["openrgb:2a3b4c5d#z0"],"plays":{"type":"effect","effect":"solid","props":{}},"mode":"flow_across","brightness":100}]},{"id":"scene:other","name":"Other","rev":1,"groups":[]},{"id":"scene:retired","name":"Retired","rev":1,"groups":[]}]}}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `ExportBundle`.

| Field | Type | What it holds |
| --- | --- | --- |
| `schema_version` | `number` | The format version of the saved file. |
| `exported_at` | `string` | The time the bundle was exported. |
| `app_version` | `string` | The version of the build that wrote the bundle. |
| `settings` | `Settings` | The saved application settings. |
| `layouts` | `LayoutsFile` | The saved layouts in the bundle. |
| `zones` | `ZonesFile` | The saved LED counts in the bundle. |
| `effects` | `EffectsFile` | The saved effects in the bundle. |
| `fans` | `FansFile` | The saved fan settings in the bundle. |
| `displays` | `array of DisplayConfig` | The numeric displays' configs. They are absent in bundles from before displays, and import then replaces the saved display configs with these, like the other files. Optional. |
| `stacks` | `array of Stack` | The saved effect stacks. They are absent in bundles from before stacks, and import then replaces the saved stacks with these, like the other files. Optional. |
| `lights` | `LightsFile` | The saved lights. They are absent in older bundles, and import then migrates their layouts. Optional. |
| `scenes` | `ScenesFile` | The saved scenes. They are absent in older bundles, as the lights are. Optional. |
| `patterns` | `array of Pattern` | The saved patterns. They are absent in older bundles. Optional. |

**Answer body**: `JSON object`.

The fields the daemon sends: the bundle was imported.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the bundle was imported. |

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

Validates and imports a configuration bundle. The bundle replaces every saved setting, scene, light, fan, display, stack and pattern with the one it carries, so export the current configuration first.

```json
{
  "schema_version": 1,
  "exported_at": "2026-11-21T12:00:00Z",
  "app_version": "0.18.1",
  "settings": {
    "fps": 30,
    "black_on_lock": false,
    "on_quit": "black",
    "boot_color": "#1b2c3d",
    "brightness": 80,
    "power": "running",
    "warmth": 6500
  },
  "layouts": {
    "schema_version": 1,
    "active": "",
    "layouts": []
  },
  "zones": {
    "schema_version": 1,
    "sizes": {}
  },
  "effects": {
    "schema_version": 1,
    "saved": {},
    "presets": {},
    "favorites": []
  },
  "fans": {
    "schema_version": 1,
    "floor_percent": 20,
    "active_profile": "Default",
    "profiles": [
      {
        "name": "Default",
        "headers": {}
      }
    ],
    "boot_color": "#1b2c3d",
    "hot_limits": {
      "cpu_c": null,
      "gpu_c": null,
      "board_c": null
    }
  },
  "displays": [
    {
      "device": "display:5f6a:7b8c",
      "source": {
        "kind": "api"
      },
      "unit": "celsius",
      "decimals": 0,
      "blink_above": null
    }
  ],
  "stacks": [
    {
      "id": "stack:example",
      "layers": []
    },
    {
      "id": "stack:other",
      "layers": []
    }
  ],
  "patterns": [
    {
      "id": "pattern:Example",
      "built_on": {
        "led_count": 4,
        "arrangement": "ring"
      },
      "frames": [
        {
          "colors": [
            "#d21e28",
            "#0f9d58",
            "#1a73e8",
            "#fbbc04"
          ],
          "hold": 0
        }
      ],
      "motion": {
        "type": "none"
      }
    },
    {
      "id": "pattern:Other",
      "built_on": {
        "led_count": 4,
        "arrangement": "ring"
      },
      "frames": [
        {
          "colors": [
            "#101010",
            "#202020",
            "#303030",
            "#404040"
          ],
          "hold": 0
        }
      ],
      "motion": {
        "type": "none"
      }
    }
  ],
  "lights": {
    "schema_version": 1,
    "chains": [],
    "lights": [
      {
        "id": "openrgb:2a3b4c5d#z0",
        "device_id": "openrgb:2a3b4c5d",
        "zone_index": 0,
        "arrangement": "strip",
        "reversed": false
      },
      {
        "id": "openrgb:6e7f8091#z0",
        "device_id": "openrgb:6e7f8091",
        "zone_index": 0,
        "arrangement": "strip",
        "reversed": false
      },
      {
        "id": "openrgb:HID:VID_3151&PID_4015#z0",
        "device_id": "openrgb:HID:VID_3151&PID_4015",
        "zone_index": 0,
        "arrangement": "strip",
        "reversed": false,
        "name": "Old keyboard lights"
      }
    ]
  },
  "scenes": {
    "schema_version": 1,
    "active": "everything",
    "scenes": [
      {
        "id": "everything",
        "name": "Everything",
        "rev": 1,
        "groups": [
          {
            "id": "everything",
            "name": "Everything",
            "members": [
              "openrgb:2a3b4c5d#z0"
            ],
            "plays": {
              "type": "effect",
              "effect": "solid",
              "props": {}
            },
            "mode": "flow_across",
            "brightness": 100
          }
        ]
      },
      {
        "id": "scene:other",
        "name": "Other",
        "rev": 1,
        "groups": []
      },
      {
        "id": "scene:retired",
        "name": "Retired",
        "rev": 1,
        "groups": []
      }
    ]
  }
}
```

// Generated from the Rust model (crates/ubot-core/src/model.rs) by the `contract` test of src-tauri.
// Do not edit: run `UBOT_UPDATE_CONTRACT=1 cargo test -p ubot-manager contract` instead.
// contract.test.ts type-checks this file against types.ts (SPEC §7).

import type { GitTestResult, RosTestResult, TaskLogEvent } from "./backend";
import type {
  ApplyOptions, Change, EngineInfo, EnvCheck, Plan, SessionStatus, Settings, Snapshot, Task
} from "./types";

export const snapshots: Snapshot[] = [
  {
    "project": {
      "path": "E:/Sims/MyRobotSim/MyRobotSim.uproject",
      "name": "MyRobotSim",
      "dir": "E:/Sims/MyRobotSim",
      "engineAssociation": "5.5",
      "engine": {
        "id": "5.5",
        "version": "5.5.4",
        "root": "D:/Program Files/Epic Games/UE_5.5",
        "kind": "launcher"
      },
      "hasCode": false,
      "buildTarget": "MyRobotSimEditor",
      "editor": {
        "running": true,
        "pid": 4242
      }
    },
    "packages": [
      {
        "name": "UBotPackage0",
        "friendlyName": "uBot Package 0",
        "description": {
          "en": "A package.",
          "zh-CN": "一个 Package。"
        },
        "layer": "Foundation",
        "state": "enabled",
        "installedVersion": "0.1.0",
        "latestVersion": "0.2.0",
        "versions": [
          "0.2.0",
          "0.1.0"
        ],
        "plannedVersion": null,
        "updateAvailable": true,
        "location": "Plugins/uBot/uBotPackage0",
        "git": {
          "isCheckout": true,
          "ref": "v0.1.0",
          "dirty": false
        },
        "repository": "https://github.com/AstreoX/uBotPackage0.git",
        "webUrl": "https://github.com/AstreoX/uBotPackage0",
        "docsUrl": null,
        "indexSource": "ubot",
        "requires": [
          {
            "name": "Dependency0",
            "constraint": "^0.1.0",
            "optional": false,
            "kind": "package",
            "status": "satisfied",
            "installedVersion": "0.1.0"
          },
          {
            "name": "Dependency1",
            "constraint": "^0.1.0",
            "optional": true,
            "kind": "enginePlugin",
            "status": "versionMismatch",
            "installedVersion": "0.1.0"
          },
          {
            "name": "Dependency2",
            "constraint": "^0.1.0",
            "optional": false,
            "kind": "package",
            "status": "notInstalled",
            "installedVersion": null
          },
          {
            "name": "Dependency3",
            "constraint": "^0.1.0",
            "optional": true,
            "kind": "enginePlugin",
            "status": "disabled",
            "installedVersion": "0.1.0"
          },
          {
            "name": "Dependency4",
            "constraint": "^0.1.0",
            "optional": false,
            "kind": "package",
            "status": "missing",
            "installedVersion": "0.1.0"
          }
        ],
        "provides": [
          "Sensor.IMU"
        ],
        "tags": [
          "sensor"
        ],
        "problemIds": [
          "requires:UBotPackage0:Dependency1"
        ]
      },
      {
        "name": "UBotPackage1",
        "friendlyName": "uBot Package 1",
        "description": {
          "en": "A package.",
          "zh-CN": null
        },
        "layer": "Capability",
        "state": "disabled",
        "installedVersion": "0.1.0",
        "latestVersion": "0.2.0",
        "versions": [
          "0.2.0",
          "0.1.0"
        ],
        "plannedVersion": null,
        "updateAvailable": true,
        "location": "Plugins/uBot/uBotPackage1",
        "git": {
          "isCheckout": true,
          "ref": null,
          "dirty": true
        },
        "repository": "https://github.com/AstreoX/uBotPackage1.git",
        "webUrl": "https://github.com/AstreoX/uBotPackage1",
        "docsUrl": null,
        "indexSource": "ubot",
        "requires": [],
        "provides": [
          "Sensor.IMU"
        ],
        "tags": [
          "sensor"
        ],
        "problemIds": []
      },
      {
        "name": "UBotPackage2",
        "friendlyName": "uBot Package 2",
        "description": {
          "en": "A package.",
          "zh-CN": "一个 Package。"
        },
        "layer": "Adapter",
        "state": "notInstalled",
        "installedVersion": null,
        "latestVersion": "0.2.0",
        "versions": [
          "0.2.0",
          "0.1.0"
        ],
        "plannedVersion": null,
        "updateAvailable": false,
        "location": null,
        "git": null,
        "repository": "https://github.com/AstreoX/uBotPackage2.git",
        "webUrl": "https://github.com/AstreoX/uBotPackage2",
        "docsUrl": null,
        "indexSource": "ubot",
        "requires": [],
        "provides": [
          "Sensor.IMU"
        ],
        "tags": [
          "sensor"
        ],
        "problemIds": []
      },
      {
        "name": "UBotPackage3",
        "friendlyName": "uBot Package 3",
        "description": {
          "en": "A package.",
          "zh-CN": null
        },
        "layer": "Content",
        "state": "planned",
        "installedVersion": null,
        "latestVersion": null,
        "versions": [],
        "plannedVersion": "0.3.0",
        "updateAvailable": false,
        "location": null,
        "git": null,
        "repository": "https://github.com/AstreoX/uBotPackage3.git",
        "webUrl": "https://github.com/AstreoX/uBotPackage3",
        "docsUrl": null,
        "indexSource": "ubot",
        "requires": [],
        "provides": [
          "Sensor.IMU"
        ],
        "tags": [
          "sensor"
        ],
        "problemIds": []
      },
      {
        "name": "UBotPackage4",
        "friendlyName": "uBot Package 4",
        "description": {
          "en": "A package.",
          "zh-CN": "一个 Package。"
        },
        "layer": "Unknown",
        "state": "enabled",
        "installedVersion": "0.1.0",
        "latestVersion": "0.2.0",
        "versions": [
          "0.2.0",
          "0.1.0"
        ],
        "plannedVersion": null,
        "updateAvailable": true,
        "location": "Plugins/uBot/uBotPackage4",
        "git": {
          "isCheckout": true,
          "ref": null,
          "dirty": false
        },
        "repository": "https://github.com/AstreoX/uBotPackage4.git",
        "webUrl": "https://github.com/AstreoX/uBotPackage4",
        "docsUrl": null,
        "indexSource": "ubot",
        "requires": [],
        "provides": [
          "Sensor.IMU"
        ],
        "tags": [
          "sensor"
        ],
        "problemIds": []
      }
    ],
    "plugins": [
      {
        "name": "UBotCore",
        "enabled": true,
        "location": "Plugins/uBot/uBotCore",
        "version": "0.1.0",
        "isUBot": true
      },
      {
        "name": "AgentSensorCore",
        "enabled": false,
        "location": "Plugins/AgentSensorCore",
        "version": null,
        "isUBot": false
      }
    ],
    "problems": [
      {
        "id": "problem-0",
        "scope": "packages",
        "group": "UBotPackage0",
        "severity": "error",
        "message": {
          "code": "problem.requiresVersion",
          "params": {
            "dep": "UBotCore",
            "installed": "0.1.0",
            "package": "UBotPackage0",
            "range": "^0.2.0"
          }
        },
        "facts": [
          {
            "label": {
              "code": "fact.package",
              "params": {}
            },
            "value": "UBotPackage0 0.1.0",
            "valueMsg": null
          },
          {
            "label": {
              "code": "fact.time",
              "params": {}
            },
            "value": "2026-10-10T08:00:00Z",
            "valueMsg": {
              "code": "session.lastPie",
              "params": {}
            }
          }
        ],
        "fixes": [
          {
            "label": {
              "code": "fix.ignore",
              "params": {}
            },
            "primary": true,
            "action": {
              "kind": "changes",
              "changes": [
                {
                  "kind": "update",
                  "package": "UBotCore",
                  "version": "0.2.0"
                }
              ]
            }
          },
          {
            "label": {
              "code": "fix.ignore",
              "params": {}
            },
            "primary": false,
            "action": {
              "kind": "ignore"
            }
          }
        ]
      },
      {
        "id": "problem-1",
        "scope": "session",
        "group": "UBotPackage0",
        "severity": "warning",
        "message": {
          "code": "problem.requiresVersion",
          "params": {
            "dep": "UBotCore",
            "installed": "0.1.0",
            "package": "UBotPackage0",
            "range": "^0.2.0"
          }
        },
        "facts": [
          {
            "label": {
              "code": "fact.package",
              "params": {}
            },
            "value": "UBotPackage0 0.1.0",
            "valueMsg": null
          },
          {
            "label": {
              "code": "fact.time",
              "params": {}
            },
            "value": "2026-10-10T08:00:00Z",
            "valueMsg": {
              "code": "session.lastPie",
              "params": {}
            }
          }
        ],
        "fixes": [
          {
            "label": {
              "code": "fix.ignore",
              "params": {}
            },
            "primary": false,
            "action": {
              "kind": "openSettings",
              "section": "appearance"
            }
          },
          {
            "label": {
              "code": "fix.ignore",
              "params": {}
            },
            "primary": false,
            "action": {
              "kind": "openSettings",
              "section": "index"
            }
          }
        ]
      },
      {
        "id": "problem-2",
        "scope": "packages",
        "group": "UBotPackage0",
        "severity": "info",
        "message": {
          "code": "problem.requiresVersion",
          "params": {
            "dep": "UBotCore",
            "installed": "0.1.0",
            "package": "UBotPackage0",
            "range": "^0.2.0"
          }
        },
        "facts": [
          {
            "label": {
              "code": "fact.package",
              "params": {}
            },
            "value": "UBotPackage0 0.1.0",
            "valueMsg": null
          },
          {
            "label": {
              "code": "fact.time",
              "params": {}
            },
            "value": "2026-10-10T08:00:00Z",
            "valueMsg": {
              "code": "session.lastPie",
              "params": {}
            }
          }
        ],
        "fixes": [
          {
            "label": {
              "code": "fix.ignore",
              "params": {}
            },
            "primary": false,
            "action": {
              "kind": "openSettings",
              "section": "install"
            }
          },
          {
            "label": {
              "code": "fix.ignore",
              "params": {}
            },
            "primary": false,
            "action": {
              "kind": "openSettings",
              "section": "build"
            }
          },
          {
            "label": {
              "code": "fix.ignore",
              "params": {}
            },
            "primary": false,
            "action": {
              "kind": "openSettings",
              "section": "ros"
            }
          }
        ]
      }
    ],
    "index": [
      {
        "id": "ubot",
        "name": "uBotCore",
        "location": "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json",
        "builtin": true,
        "enabled": true,
        "ok": false,
        "fromCache": true,
        "error": "HTTP 404 Not Found",
        "packages": 9,
        "fetchedAt": "2026-10-10T08:00:00Z"
      },
      {
        "id": "local",
        "name": "Local copy",
        "location": "Plugins/uBot/uBotCore/Index/index.json",
        "builtin": true,
        "enabled": true,
        "ok": true,
        "fromCache": false,
        "error": null,
        "packages": 9,
        "fetchedAt": null
      }
    ],
    "recipes": [
      {
        "id": "ros-sensors",
        "name": {
          "en": "ROS 2 sensors",
          "zh-CN": "ROS 2 传感器"
        },
        "description": {
          "en": "Sensors published to ROS 2.",
          "zh-CN": null
        },
        "packages": [
          "UBotCore",
          "UBotSensor",
          "UBotROS"
        ],
        "checks": [
          "rosBridge"
        ],
        "available": true,
        "user": false
      },
      {
        "id": "mine",
        "name": {
          "en": "Mine",
          "zh-CN": null
        },
        "description": {
          "en": "",
          "zh-CN": null
        },
        "packages": [
          "UBotCore"
        ],
        "checks": [],
        "available": false,
        "user": true
      }
    ],
    "session": {
      "kind": "PIE",
      "startedAt": "2026-10-10T08:00:00Z",
      "endedAt": "2026-10-10T08:05:00Z",
      "engineVersion": "5.5.4"
    },
    "syncing": true
  },
  {
    "project": null,
    "packages": [],
    "plugins": [],
    "problems": [],
    "index": [],
    "recipes": [],
    "session": null,
    "syncing": false
  }
];

export const sessions: SessionStatus[] = [
  {
    "kind": "PIE",
    "startedAt": "2026-10-10T08:00:00Z",
    "endedAt": "2026-10-10T08:05:00Z",
    "engineVersion": "5.5.4"
  },
  {
    "kind": "Editor",
    "startedAt": "2026-10-10T08:00:00Z",
    "endedAt": null,
    "engineVersion": "5.5.4"
  },
  {
    "kind": "Game",
    "startedAt": "2026-10-10T08:00:00Z",
    "endedAt": "2026-10-10T08:05:00Z",
    "engineVersion": "5.5.4"
  }
];

export const plans: Plan[] = [
  {
    "changes": [
      {
        "kind": "install",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "update",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "enable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "disable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "uninstall",
        "package": "UBotSensor",
        "version": null
      }
    ],
    "steps": [
      {
        "id": "step-1",
        "n": 1,
        "group": "download",
        "kind": "clone",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": false
      },
      {
        "id": "step-2",
        "n": 2,
        "group": "download",
        "kind": "checkout",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": true
      },
      {
        "id": "step-3",
        "n": 3,
        "group": "download",
        "kind": "enablePackage",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": true
      },
      {
        "id": "step-4",
        "n": 4,
        "group": "project",
        "kind": "disablePackage",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": true
      },
      {
        "id": "step-5",
        "n": 5,
        "group": "project",
        "kind": "enableEnginePlugin",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": true
      },
      {
        "id": "step-6",
        "n": 6,
        "group": "build",
        "kind": "removePackage",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": true
      },
      {
        "id": "step-7",
        "n": 7,
        "group": "build",
        "kind": "build",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": true
      },
      {
        "id": "step-8",
        "n": 8,
        "group": "after",
        "kind": "runTests",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": true
      },
      {
        "id": "step-9",
        "n": 9,
        "group": "after",
        "kind": "openEditor",
        "title": {
          "code": "step.clone",
          "params": {
            "package": "UBotSensor",
            "version": "0.2.0"
          }
        },
        "detail": {
          "code": "step.clone.detail",
          "params": {
            "dest": "Plugins/uBot/uBotSensor",
            "repo": "https://github.com/AstreoX/uBotSensor.git"
          }
        },
        "needsEditorClosed": true
      }
    ],
    "needsBuild": true,
    "editorRunning": true,
    "errors": [
      {
        "code": "plan.breaksDependent",
        "params": {
          "dependent": "UBotSensor",
          "package": "UBotCore",
          "range": "^0.1.0"
        }
      }
    ],
    "warnings": [
      {
        "code": "plan.onBranch",
        "params": {
          "branch": "main",
          "package": "UBotCore"
        }
      }
    ]
  },
  {
    "changes": [],
    "steps": [],
    "needsBuild": false,
    "editorRunning": false,
    "errors": [],
    "warnings": []
  }
];

export const tasks: Task[] = [
  {
    "id": "task-pending",
    "title": {
      "code": "task.apply",
      "params": {
        "project": "MyRobotSim"
      }
    },
    "project": "E:/Sims/MyRobotSim/MyRobotSim.uproject",
    "status": "pending",
    "startedAt": "2026-10-10T08:00:00Z",
    "durationMs": null,
    "steps": [
      {
        "id": "step-1",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "pending",
        "startedAt": null,
        "durationMs": null,
        "progress": null,
        "logLines": 0
      },
      {
        "id": "step-2",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "waiting",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": null,
        "progress": null,
        "logLines": 10
      },
      {
        "id": "step-3",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "running",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": null,
        "progress": {
          "done": 15,
          "total": 24
        },
        "logLines": 20
      },
      {
        "id": "step-4",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "done",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": 1500,
        "progress": null,
        "logLines": 30
      },
      {
        "id": "step-5",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "failed",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": 1500,
        "progress": null,
        "logLines": 40
      },
      {
        "id": "step-6",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "skipped",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": 1500,
        "progress": null,
        "logLines": 50
      },
      {
        "id": "step-7",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "canceled",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": 1500,
        "progress": null,
        "logLines": 60
      }
    ],
    "error": null,
    "changes": [
      {
        "kind": "install",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "update",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "enable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "disable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "uninstall",
        "package": "UBotSensor",
        "version": null
      }
    ]
  },
  {
    "id": "task-waiting",
    "title": {
      "code": "task.apply",
      "params": {
        "project": "MyRobotSim"
      }
    },
    "project": "E:/Sims/MyRobotSim/MyRobotSim.uproject",
    "status": "waiting",
    "startedAt": "2026-10-10T08:00:00Z",
    "durationMs": null,
    "steps": [
      {
        "id": "step-2",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "waiting",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": null,
        "progress": null,
        "logLines": 10
      }
    ],
    "error": null,
    "changes": [
      {
        "kind": "install",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "update",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "enable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "disable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "uninstall",
        "package": "UBotSensor",
        "version": null
      }
    ]
  },
  {
    "id": "task-running",
    "title": {
      "code": "task.apply",
      "params": {
        "project": "MyRobotSim"
      }
    },
    "project": "E:/Sims/MyRobotSim/MyRobotSim.uproject",
    "status": "running",
    "startedAt": "2026-10-10T08:00:00Z",
    "durationMs": null,
    "steps": [
      {
        "id": "step-3",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "running",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": null,
        "progress": {
          "done": 15,
          "total": 24
        },
        "logLines": 20
      }
    ],
    "error": null,
    "changes": [
      {
        "kind": "install",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "update",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "enable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "disable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "uninstall",
        "package": "UBotSensor",
        "version": null
      }
    ]
  },
  {
    "id": "task-done",
    "title": {
      "code": "task.apply",
      "params": {
        "project": "MyRobotSim"
      }
    },
    "project": "E:/Sims/MyRobotSim/MyRobotSim.uproject",
    "status": "done",
    "startedAt": "2026-10-10T08:00:00Z",
    "durationMs": 64000,
    "steps": [
      {
        "id": "step-4",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "done",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": 1500,
        "progress": null,
        "logLines": 30
      }
    ],
    "error": null,
    "changes": [
      {
        "kind": "install",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "update",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "enable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "disable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "uninstall",
        "package": "UBotSensor",
        "version": null
      }
    ]
  },
  {
    "id": "task-failed",
    "title": {
      "code": "task.apply",
      "params": {
        "project": "MyRobotSim"
      }
    },
    "project": "E:/Sims/MyRobotSim/MyRobotSim.uproject",
    "status": "failed",
    "startedAt": "2026-10-10T08:00:00Z",
    "durationMs": 64000,
    "steps": [
      {
        "id": "step-5",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "failed",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": 1500,
        "progress": null,
        "logLines": 40
      }
    ],
    "error": "Build failed.",
    "changes": [
      {
        "kind": "install",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "update",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "enable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "disable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "uninstall",
        "package": "UBotSensor",
        "version": null
      }
    ]
  },
  {
    "id": "task-skipped",
    "title": {
      "code": "task.apply",
      "params": {
        "project": "MyRobotSim"
      }
    },
    "project": "E:/Sims/MyRobotSim/MyRobotSim.uproject",
    "status": "skipped",
    "startedAt": "2026-10-10T08:00:00Z",
    "durationMs": 64000,
    "steps": [
      {
        "id": "step-6",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "skipped",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": 1500,
        "progress": null,
        "logLines": 50
      }
    ],
    "error": null,
    "changes": [
      {
        "kind": "install",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "update",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "enable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "disable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "uninstall",
        "package": "UBotSensor",
        "version": null
      }
    ]
  },
  {
    "id": "task-canceled",
    "title": {
      "code": "task.apply",
      "params": {
        "project": "MyRobotSim"
      }
    },
    "project": "E:/Sims/MyRobotSim/MyRobotSim.uproject",
    "status": "canceled",
    "startedAt": "2026-10-10T08:00:00Z",
    "durationMs": 64000,
    "steps": [
      {
        "id": "step-7",
        "title": {
          "code": "step.build",
          "params": {
            "target": "MyRobotSimEditor"
          }
        },
        "status": "canceled",
        "startedAt": "2026-10-10T08:00:00Z",
        "durationMs": 1500,
        "progress": null,
        "logLines": 60
      }
    ],
    "error": null,
    "changes": [
      {
        "kind": "install",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "update",
        "package": "UBotSensor",
        "version": "0.2.0"
      },
      {
        "kind": "enable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "disable",
        "package": "UBotSensor",
        "version": null
      },
      {
        "kind": "uninstall",
        "package": "UBotSensor",
        "version": null
      }
    ]
  }
];

export const taskLogEvents: TaskLogEvent[] = [
  {
    "taskId": "task-1",
    "stepId": "step-1",
    "lines": [
      "[15/24] Compile Module.UBotSensor.cpp"
    ]
  }
];

export const settings: Settings[] = [
  {
    "language": "en",
    "theme": "dark",
    "index": {
      "sources": [
        {
          "id": "ubot",
          "name": "uBotCore",
          "location": "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json",
          "enabled": true,
          "builtin": true
        },
        {
          "id": "user-1",
          "name": "Team mirror",
          "location": "//server/share/index.json",
          "enabled": true,
          "builtin": false
        }
      ],
      "syncOnStart": true,
      "showPlanned": true
    },
    "install": {
      "directory": "Plugins/uBot",
      "git": "git",
      "usePrebuilt": false
    },
    "build": {
      "engineId": null,
      "configuration": "Development",
      "waitForEditor": true,
      "reopenEditor": true,
      "runTests": false
    },
    "ros": {
      "host": "127.0.0.1",
      "port": 8268,
      "checkOnRefresh": true
    },
    "recentProjects": [
      "E:/Sims/MyRobotSim/MyRobotSim.uproject"
    ],
    "lastProject": "E:/Sims/MyRobotSim/MyRobotSim.uproject"
  },
  {
    "language": "zh-CN",
    "theme": "light",
    "index": {
      "sources": [
        {
          "id": "ubot",
          "name": "uBotCore",
          "location": "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json",
          "enabled": true,
          "builtin": true
        },
        {
          "id": "user-1",
          "name": "Team mirror",
          "location": "//server/share/index.json",
          "enabled": false,
          "builtin": false
        }
      ],
      "syncOnStart": false,
      "showPlanned": true
    },
    "install": {
      "directory": "Plugins/uBot",
      "git": "git",
      "usePrebuilt": false
    },
    "build": {
      "engineId": "5.5",
      "configuration": "DebugGame",
      "waitForEditor": true,
      "reopenEditor": true,
      "runTests": false
    },
    "ros": {
      "host": "127.0.0.1",
      "port": 8268,
      "checkOnRefresh": true
    },
    "recentProjects": [
      "E:/Sims/MyRobotSim/MyRobotSim.uproject"
    ],
    "lastProject": null
  },
  {
    "language": "en",
    "theme": "system",
    "index": {
      "sources": [
        {
          "id": "ubot",
          "name": "uBotCore",
          "location": "https://raw.githubusercontent.com/AstreoX/uBotCore/main/Index/index.json",
          "enabled": true,
          "builtin": true
        },
        {
          "id": "user-1",
          "name": "Team mirror",
          "location": "//server/share/index.json",
          "enabled": true,
          "builtin": false
        }
      ],
      "syncOnStart": false,
      "showPlanned": true
    },
    "install": {
      "directory": "Plugins/uBot",
      "git": "git",
      "usePrebuilt": false
    },
    "build": {
      "engineId": null,
      "configuration": "Development",
      "waitForEditor": true,
      "reopenEditor": false,
      "runTests": true
    },
    "ros": {
      "host": "127.0.0.1",
      "port": 8268,
      "checkOnRefresh": true
    },
    "recentProjects": [
      "E:/Sims/MyRobotSim/MyRobotSim.uproject"
    ],
    "lastProject": null
  }
];

export const engines: EngineInfo[] = [
  {
    "id": "5.5",
    "version": "5.5.4",
    "root": "D:/Program Files/Epic Games/UE_5.5",
    "kind": "launcher"
  },
  {
    "id": "{8A1E5B5C-0000-4000-8000-000000000000}",
    "version": "5.5.4",
    "root": "D:/UE/Source",
    "kind": "source"
  },
  {
    "id": "D:/UE/Custom",
    "version": "5.5.4",
    "root": "D:/UE/Custom",
    "kind": "custom"
  }
];

export const envChecks: EnvCheck[] = [
  {
    "id": "git",
    "ok": true,
    "detail": "git version 2.47.0"
  },
  {
    "id": "vs2022",
    "ok": false,
    "detail": "git version 2.47.0"
  },
  {
    "id": "engine",
    "ok": true,
    "detail": "git version 2.47.0"
  },
  {
    "id": "rosBridge",
    "ok": null,
    "detail": null
  }
];

export const changes: Change[] = [
  {
    "kind": "install",
    "package": "UBotSensor",
    "version": "0.2.0"
  },
  {
    "kind": "update",
    "package": "UBotSensor",
    "version": "0.2.0"
  },
  {
    "kind": "enable",
    "package": "UBotSensor",
    "version": null
  },
  {
    "kind": "disable",
    "package": "UBotSensor",
    "version": null
  },
  {
    "kind": "uninstall",
    "package": "UBotSensor",
    "version": null
  }
];

export const applyOptions: ApplyOptions = {
  "build": true,
  "reopenEditor": true,
  "runTests": false
};

export const gitTests: GitTestResult[] = [
  {
    "ok": true,
    "version": "2.47.0",
    "error": null
  },
  {
    "ok": false,
    "version": null,
    "error": "program not found"
  }
];

export const rosTests: RosTestResult[] = [
  {
    "ok": true,
    "error": null
  },
  {
    "ok": false,
    "error": "connection refused"
  }
];

"""The loader: config.yaml, config.d/ and the plugin manifests, and what they install.

A loader outside the core reads each plugin's manifest and assembles its tools, middleware,
commands, skills, rules, knob-offset bounds and extension points; the core sees only
percepts, actions and the three hooks (middleware.md §5). Nothing here is stored in the
database: the configuration files are the source of truth (D59), and the program writes
them only on the owner's command, narrowly and after a backup.
"""

from __future__ import annotations

import importlib
import importlib.util
import json
import pathlib
import re
import shutil
import sys
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

import yaml

from monoagens.core.hooks import Continue, Middleware
from monoagens.infra import config as config_module
from monoagens.infra.store import append_event
from monoagens.middleware import contract, policy, registry, roles
from monoagens.middleware.registry import CommandEntry, SkillEntry

MANIFEST_VERSION = 1
CONFIG_D_VERSION = 1
REQUIRED_MANIFEST = ("manifest_version", "name", "version", "description")
OPTIONAL_MANIFEST = ("dependencies", "optional_dependencies", "state_namespace", "features",
                     "tools", "settings", "permissions", "role_settings", "extension_points",
                     "skills", "rules", "knob_offsets", "requires")
CONFIG_D_KEYS = ("version", "features", "options", "permissions", "role_settings")
FEATURE_LISTS = ("tools", "commands", "middleware")
OPTION_TYPES: dict[str, tuple[type, ...]] = {"string": (str,), "int": (int,),
                                             "number": (int, float), "boolean": (bool,),
                                             "array": (list,), "object": (dict,)}
PLUGINS = "plugins"
FRONT_MATTER = re.compile(r"^---\n(.*?)\n---\n?(.*)$", re.S)


@dataclass
class BootReport:
    """What the loader did, in the order it happened: the boot report the owner reads."""

    lines: list[str] = field(default_factory=list)

    def add(self, line: str) -> None:
        """Add one report line."""
        self.lines.append(line)

    def render(self) -> str:
        """Render the report; a clean boot says so in one line."""
        return "boot report:\n" + "\n".join(self.lines) if self.lines else "boot report: clean"


@dataclass(frozen=True)
class Loaded:
    """Everything one load produced, ready to be installed on a kernel."""

    report: BootReport
    plugins: tuple[str, ...]
    adapters: tuple[Any, ...]
    permissions: dict[str, str | None]
    knob_bounds: dict[str, dict[str, tuple[float, float]]]
    apis: tuple["PluginAPI", ...] = ()
    roles: Any = None


@dataclass
class PluginAPI:
    """What one plugin may do: tools, commands, skills, rules, middleware, points."""

    name: str
    manifest: dict[str, Any]
    options: dict[str, Any]
    features: dict[str, list[str]]
    log: Any = None
    points: dict[str, dict[str, Any]] = field(default_factory=dict)
    tools: list[tuple[Any, dict[str, Any]]] = field(default_factory=list)
    commands: list[CommandEntry] = field(default_factory=list)
    skills: list[SkillEntry] = field(default_factory=list)
    rules: list[str] = field(default_factory=list)
    middleware: list[tuple[str, str, Middleware]] = field(default_factory=list)
    checks: list[Any] = field(default_factory=list)

    def add_tool(self, tool: Any, facts: Mapping[str, Any] | None = None) -> None:
        """Add one tool; its facts come from the manifest unless given."""
        self.tools.append((tool, dict(facts if facts is not None
                                      else self.manifest.get("tools") or {})))

    def add_command(self, name: str, description: str, handler: Callable[..., Any], *,
                    permission: str) -> None:
        """Add one command a person may type."""
        self.commands.append(CommandEntry(name, description, permission, handler))

    def add_rule(self, text: str) -> None:
        """Add one standing rule, shown in `<self>` in install order."""
        self.rules.append(str(text).strip())

    def add_middleware(self, hook: str, name: str, middleware: Middleware) -> None:
        """Add one middleware to one hook; the installer names it `<plugin>-<name>`."""
        self.middleware.append((str(hook), str(name), middleware))

    def open_point(self, point: str) -> None:
        """Open one extension point; a point opens once."""
        if point in self.points:
            raise ValueError(f"extension point {point} is already open")
        self.points[point] = {}

    def fill_point(self, point: str, key: str, fn: Any) -> None:
        """Fill one open extension point under one key (a file suffix, a body name)."""
        if point not in self.points:
            raise ValueError(f"plugin {self.name}: no such extension point {point}")
        if key in self.points[point]:
            raise ValueError(f"extension point {point} already has {key}")
        self.points[point][key] = fn

    def point(self, point: str) -> dict[str, Any]:
        """Return the current fillings of one extension point, empty when unknown."""
        return dict(self.points.get(point) or {})

    def is_blocked_address(self, address: str) -> bool:
        """Return whether one address is private, reserved or otherwise off limits."""
        return policy.is_blocked_address(address)


class Loader:
    """Read the deployment files and assemble the plugins they enable."""

    def __init__(self, root: pathlib.Path, *, env: Mapping[str, str] | None = None) -> None:
        """Point the loader at one deployment root and one environment."""
        self.root = pathlib.Path(root)
        self.env = dict(env or {})
        self._slots: dict[tuple[str, str], _Slot] = {}

    # ------------------------------------------------------------- loading

    def load(self, settings: Mapping[str, Any], *, notes: Sequence[str] = (),
             who: str = "the owner") -> Loaded:
        """Read config.yaml's plugins, config.d/ and the manifests, then call each register."""
        self.who = who
        report = BootReport(list(notes))
        installed = self._installed(settings)
        config_d = self._config_d(installed, report)
        manifests = {name: self._manifest(name) for name in installed
                     if bool((installed[name] or {}).get("enabled", False))}
        order = self._order(installed, manifests, report)
        self._points: dict[str, dict[str, Any]] = {}
        apis: list[Any] = []
        unavailable: list[str] = []
        for name in order:
            manifest = manifests[name]
            if not self._requirements(name, manifest, report):
                unavailable.append(name)
                continue
            effective = self._features(name, manifest, config_d.get(name) or {}, report)
            api = PluginAPI(name=name, manifest=manifest, points=self._points,
                            options=self._options(name, manifest, config_d.get(name) or {}),
                            features=effective)
            for point in (manifest.get("extension_points") or {}).get("opens") or ():
                api.open_point(str(point))
            self._register(name, api, report)
            apis.append(api)
        loaded = Loaded(report=report, plugins=tuple(api.name for api in apis),
                        adapters=self.adapters(settings, report),
                        permissions=self._permissions(manifests, config_d),
                        knob_bounds=self._knob_bounds(manifests), apis=tuple(apis),
                        roles=self._roles(settings, manifests, config_d, report))
        if unavailable:
            report.add(f"plugins unavailable: {', '.join(sorted(unavailable))}")
        return loaded

    def _installed(self, settings: Mapping[str, Any]) -> dict[str, dict[str, Any]]:
        """Return the plugins section as a map (a version 1 list reads as all enabled)."""
        installed = settings.get(PLUGINS) or {}
        if isinstance(installed, list):
            return {str(name): {"enabled": True} for name in installed}
        return {str(name): dict(value or {}) for name, value in dict(installed).items()}

    def _manifest(self, name: str) -> dict[str, Any]:
        """Read and check one plugin's manifest."""
        folder = self.root / PLUGINS / name
        if not folder.is_dir():
            raise ValueError(f"plugin not installed: {name}")
        manifest = config_module.read_versioned(
            folder / "plugin.yaml", key="manifest_version", supported=MANIFEST_VERSION,
            label=f"plugin {name}", upcasters={})
        self._check_manifest(name, manifest)
        return manifest

    def _check_manifest(self, name: str, manifest: Mapping[str, Any]) -> None:
        """Refuse an unknown, missing or mismatched manifest key at boot."""
        for key in manifest:
            if key not in REQUIRED_MANIFEST + OPTIONAL_MANIFEST:
                raise ValueError(f"plugin {name}: unknown manifest key {key!r}")
        for key in REQUIRED_MANIFEST:
            if key not in manifest:
                raise ValueError(f"plugin {name}: manifest needs {key}")
        if str(manifest["name"]) != name:
            raise ValueError(f"plugin {name}: manifest name is {manifest['name']!r}")
        namespace = manifest.get("state_namespace")
        if namespace is not None and str(namespace) != name:
            raise ValueError(f"plugin {name}: state_namespace must be {name!r}")

    def _config_d(self, installed: Mapping[str, Any], report: BootReport,
                  ) -> dict[str, dict[str, Any]]:
        """Read config.d/: one file per plugin, reported when it belongs to no installed one."""
        folder = self.root / "config.d"
        found: dict[str, dict[str, Any]] = {}
        for path in sorted(folder.glob("*.yaml")) if folder.is_dir() else []:
            stem = path.stem
            values = config_module.read_versioned(
                path, key="version", supported=CONFIG_D_VERSION,
                label=f"config.d/{stem}.yaml", upcasters={}, notes=report.lines)
            self._check_config_d(path, values)
            if stem not in installed:
                report.add(f"config.d/{stem}.yaml: plugin {stem} is not installed; ignored")
                continue
            if not bool((installed[stem] or {}).get("enabled", False)):
                report.add(f"config.d/{stem}.yaml: plugin is off; kept")
                continue
            found[stem] = values
        return found

    @staticmethod
    def _check_config_d(path: pathlib.Path, values: Mapping[str, Any]) -> None:
        """Refuse an unknown key in one config.d file."""
        for key in values:
            if key not in CONFIG_D_KEYS:
                raise ValueError(f"config.d/{path.stem}.yaml: unknown key {key!r}")

    def _order(self, installed: Mapping[str, Any], manifests: Mapping[str, Mapping[str, Any]],
               report: BootReport) -> list[str]:
        """Return the enabled plugins in dependency order; a cycle or a missing one fails."""
        enabled = {name: manifest for name, manifest in manifests.items()}
        order: list[str] = []
        visiting: list[str] = []

        def visit(name: str) -> None:
            if name in order:
                return
            if name in visiting:
                cycle = " -> ".join([*visiting[visiting.index(name):], name])
                raise ValueError(f"plugin cycle: {cycle}")
            visiting.append(name)
            manifest = enabled[name]
            for dependency in manifest.get("dependencies") or ():
                if str(dependency) not in enabled:
                    raise ValueError(f"plugin {name}: dependency {dependency} is not installed")
                visit(str(dependency))
            for dependency in manifest.get("optional_dependencies") or ():
                if str(dependency) in enabled:
                    visit(str(dependency))
                else:
                    report.add(f"plugin {name}: optional dependency {dependency} is not "
                               "installed")
            visiting.pop()
            order.append(name)

        for name in sorted(enabled):
            visit(name)
        for name in installed:
            if name not in enabled:
                report.add(f"plugin {name}: off")
        return order

    @staticmethod
    def _requirements(name: str, manifest: Mapping[str, Any], report: BootReport) -> bool:
        """Return whether every Python module one plugin requires can be imported."""
        for module in manifest.get("requires") or ():
            try:
                found = (str(module) in sys.modules
                         or importlib.util.find_spec(str(module)) is not None)
            except (ImportError, ValueError):
                found = False
            if not found:
                report.add(f"plugin {name}: needs {module}; install it with: "
                           f"uv pip install -r plugins/{name}/requirements.txt")
                return False
        return True

    @staticmethod
    def _options(name: str, manifest: Mapping[str, Any],
                 config_d: Mapping[str, Any]) -> dict[str, Any]:
        """Merge the manifest defaults with config.d options; a bad key or type fails."""
        schema = manifest.get("settings") or {}
        options = {key: spec.get("default") for key, spec in schema.items()}
        for key, value in (config_d.get("options") or {}).items():
            spec = schema.get(key)
            if spec is None:
                raise ValueError(f"config.d/{name}.yaml: options.{key}: not in the manifest")
            kind = str(spec.get("type") or "")
            if not isinstance(value, OPTION_TYPES.get(kind, (object,))):
                raise ValueError(f"config.d/{name}.yaml: options.{key}: expected {kind}")
            if isinstance(value, bool) and kind in ("int", "number"):
                raise ValueError(f"config.d/{name}.yaml: options.{key}: expected {kind}")
            options[key] = value
        return options

    @staticmethod
    def _features(name: str, manifest: Mapping[str, Any], config_d: Mapping[str, Any],
                  report: BootReport) -> dict[str, list[str]]:
        """Merge the feature lists: maps remove entries, a list replaces, `[]` is all off."""
        declared = {key: list(value or []) for key, value in
                    (manifest.get("features") or {}).items()}
        for key, value in (config_d.get("features") or {}).items():
            if key not in FEATURE_LISTS and key not in declared:
                raise ValueError(f"config.d/{name}.yaml: unknown feature {key!r}")
            if isinstance(value, list):
                declared[key] = list(value)
                report.add(f"config.d/{name}.yaml: {key} list replaces the manifest default")
            elif isinstance(value, dict):
                declared[key] = [item for item in declared.get(key, [])
                                 if value.get(item, True) is not False]
            else:
                raise ValueError(f"config.d/{name}.yaml: features.{key}: expected list or map")
        return declared

    @staticmethod
    def _roles(settings: Mapping[str, Any], manifests: Mapping[str, Mapping[str, Any]],
               config_d: Mapping[str, Mapping[str, Any]], report: BootReport) -> Any:
        """Resolve the role book from config.yaml, the manifests and config.d."""
        catalog = dict(roles.CORE_PERMISSIONS)
        for manifest in manifests.values():
            for name, spec in (manifest.get("permissions") or {}).items():
                catalog.setdefault(str(name), (spec or {}).get("base"))
        for values in config_d.values():
            for name, base in (values.get("permissions") or {}).items():
                catalog.setdefault(str(name), base)
        return roles.RoleBook.build({"roles": settings.get("roles") or {},
                                     "people": settings.get("people") or {},
                                     "places": settings.get("places") or {}},
                                    catalog, manifests, config_d, report)

    def adapters(self, settings: Mapping[str, Any], report: BootReport) -> tuple[Any, ...]:
        """Create every enabled adapter from `bodies:` through its `create(options, env)`."""
        bodies = settings.get("bodies") or {}
        found: list[Any] = []
        for name in sorted(bodies):
            options = bodies[name] or {}
            if not isinstance(options, dict):
                raise ValueError(f"bodies.{name}: expected a map")
            if not bool(options.get("enabled", False)):
                report.add(f"adapter {name}: off")
                continue
            module_name = str(options.get("module") or f"monoagens.adapters.{name}")
            module = importlib.import_module(module_name)
            factory = getattr(module, "create", None)
            if not callable(factory):
                raise ValueError(f"adapter {name}: {module_name} has no create function")
            env = {str(key): str(self.env.get(str(key)) or "")
                   for key in getattr(module, "REQUIRES", ())}
            for key, value in env.items():
                if not value:
                    raise ValueError(f"{name}: {key} is not set")
            adapter = factory({key: value for key, value in options.items() if key != "module"},
                              env)
            self._check_adapter(name, adapter, report)
            found.append(adapter)
        return tuple(found)

    @staticmethod
    def _check_adapter(name: str, adapter: Any, report: BootReport) -> None:
        """Refuse an adapter that cannot do its job; report what it can additionally do."""
        for part in ("capabilities", "run", "act"):
            if not callable(getattr(adapter, part, None)):
                raise ValueError(f"adapter {name}: missing required part {part!r}")
        if not isinstance(getattr(adapter, "name", None), str) or not adapter.name:
            raise ValueError(f"adapter {name}: missing required part 'name'")
        shows = tuple(getattr(adapter, "shows", ()) or ())
        unknown = [kind for kind in shows if kind not in contract.SHOWS]
        if unknown:
            raise ValueError(f"adapter {name}: cannot show {unknown[0]!r}")
        report.add(f"adapter {name}: shows {', '.join(shows) or 'nothing'}")
        for part in ("history", "place_info", "get_cursor"):
            if getattr(adapter, part, None) is not None:
                report.add(f"adapter {name}: has {part}")

    # ---------------------------------------------------------- registering

    def _register(self, name: str, api: PluginAPI, report: BootReport) -> None:
        """Import one plugin, call its register, and validate everything it brought."""
        module = self._import(name)
        register = getattr(module, "register", None)
        if not callable(register):
            raise ValueError(f"plugin {name}: no register function")
        register(api)
        for tool, facts in api.tools:
            owner = getattr(tool, "name", None) or name
            self._validate_tool(owner, tool, facts, api.manifest.get("permissions") or {})
        for entry in api.commands:
            if entry.permission and not self._declared(api, entry.permission):
                raise ValueError(f"{entry.name}: an entry without a declared permission")
        for declared in (api.manifest.get("extension_points") or {}).get("fills") or ():
            if str(declared) not in api.points:
                raise ValueError(f"plugin {name}: no such extension point {declared}")
        for skill in api.manifest.get("skills") or ():
            api.skills.append(self._skill(name, str(skill)))

    @staticmethod
    def _validate_tool(owner: str, tool: Any, facts: Mapping[str, Any],
                       catalog: Mapping[str, Any]) -> None:
        """Validate one plugin tool: facts, a declared permission, no collision."""
        declared = facts or {}
        for capability in getattr(tool, "capabilities", lambda: ())():
            full = f"{owner}.{capability['name']}"
            spec = declared.get(str(capability["name"]))
            if not spec or not spec.get("facts"):
                raise ValueError(f"{full}: a tool without facts")
            permission = str(spec.get("permission") or full)
            if permission not in catalog:
                raise ValueError(f"{full}: an entry without a declared permission")
        registry.ToolRegistry().add_tool(tool, facts, owner=owner)

    @staticmethod
    def _declared(api: PluginAPI, permission: str) -> bool:
        """Return whether a plugin declared this permission in its manifest."""
        return permission in (api.manifest.get("permissions") or {})

    def _skill(self, name: str, skill: str) -> SkillEntry:
        """Read one SKILL.md: YAML front matter, then the instructions."""
        path = self.root / PLUGINS / name / "skills" / skill / "SKILL.md"
        if not path.is_file():
            raise ValueError(f"plugin {name}: no skill {skill}")
        match = FRONT_MATTER.match(path.read_text(encoding="utf-8"))
        if match is None:
            raise ValueError(f"plugin {name}: skill {skill} has no front matter")
        head = yaml.safe_load(match.group(1)) or {}
        for key in ("name", "description", "version"):
            if key not in head:
                raise ValueError(f"plugin {name}: skill {skill} needs {key}")
        return SkillEntry(str(head["name"]), str(head["description"]), match.group(2).strip())

    def _import(self, name: str) -> Any:
        """Import plugins/<name>/__init__.py under its own module name."""
        folder = self.root / PLUGINS / name
        init = folder / "__init__.py"
        if not init.is_file():
            raise ValueError(f"plugin {name}: no __init__.py")
        module_name = f"monoagens_plugins_{name.replace('-', '_')}"
        spec = importlib.util.spec_from_file_location(
            module_name, init, submodule_search_locations=[str(folder)])
        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        spec.loader.exec_module(module)
        return module

    @staticmethod
    def _permissions(manifests: Mapping[str, Mapping[str, Any]],
                     config_d: Mapping[str, Mapping[str, Any]]) -> dict[str, str | None]:
        """Return the effective lowest tier per permission: config.d, then the manifest."""
        found: dict[str, str | None] = {}
        for name, manifest in manifests.items():
            for permission, declared in (manifest.get("permissions") or {}).items():
                found[str(permission)] = (declared or {}).get("base")
            for permission, base in (config_d.get(name, {}).get("permissions") or {}).items():
                found[str(permission)] = base
        return found

    @staticmethod
    def _knob_bounds(manifests: Mapping[str, Mapping[str, Any]]
                     ) -> dict[str, dict[str, tuple[float, float]]]:
        """Return every plugin's knob-offset bounds from its manifest."""
        bounds: dict[str, dict[str, tuple[float, float]]] = {}
        for name, manifest in manifests.items():
            offsets = manifest.get("knob_offsets") or {}
            if offsets:
                bounds[name] = {str(knob): (float(pair[0]), float(pair[1]))
                                for knob, pair in offsets.items()}
        return bounds

    # ------------------------------------------------------------ installing

    async def install(self, kernel: Any, loaded: Loaded) -> None:
        """Install everything one load produced onto a kernel, in load order."""
        install_commands(self, kernel)
        touched: set[tuple[str, str]] = set()
        for previous in getattr(kernel, "loaded_plugins", ()):
            kernel.tools.discard_owner(previous)
        for api in loaded.apis:
            for tool, facts in api.tools:
                kernel.tools.add_tool(tool, facts, owner=getattr(tool, "name", None) or api.name)
            for entry in api.commands:
                kernel.tools.add_command(_owned(entry, api.name))
            for entry in api.skills:
                kernel.tools.add_skill(_owned_skill(entry, api.name))
            for rule in api.rules:
                kernel.tools.add_rule(rule)
            for hook, name, middleware in api.middleware:
                self._slot(kernel, hook, f"{api.name}-{name}").target = middleware
                touched.add((hook, f"{api.name}-{name}"))
        for key, slot in self._slots.items():
            if key not in touched:
                slot.target = None  # the plugin that owned this place is gone
        for adapter in loaded.adapters:
            if hasattr(adapter, "get_cursor"):
                adapter.get_cursor = _cursor_for(kernel, adapter.name)
            kernel.register_adapter(adapter)
        if loaded.roles is not None:
            kernel.roles = loaded.roles
        kernel.deployment_root = self.root
        kernel.loaded_plugins = loaded.plugins
        kernel.plugin_versions = {api.name: str(api.manifest.get("version") or "")
                                  for api in loaded.apis}
        kernel.boot_report = loaded.report

    def _slot(self, kernel: Any, hook: str, name: str) -> "_Slot":
        """Return the stable slot of one plugin middleware, registering it the first time.

        A reload swaps what a slot calls instead of adding a second middleware, so the
        pipeline's name (and its order) never changes without a restart.
        """
        key = (hook, name)
        slot = self._slots.get(key)
        if slot is not None:
            return slot
        slot = _Slot()
        self._slots[key] = slot
        kernel.hooks.get(hook).add(name, slot)
        return slot


class _Slot:
    """One middleware's place in a hook; a reload swaps the target, not the place."""

    target: Middleware | None = None

    def __init__(self) -> None:
        """Start empty: a slot with no target lets everything through."""
        self.target = None

    async def __call__(self, item: Any, ctx: Any) -> Any:
        """Call the current middleware, or pass when the plugin is gone."""
        if self.target is None:
            return Continue()
        return await self.target(item, ctx)


def _cursor_for(kernel: Any, name: str) -> Callable[[], Any]:
    """Return the cursor reader one adapter asks for before its first poll."""

    async def cursor() -> Any:
        return await kernel.cursor(name)

    return cursor


def _owned(entry: CommandEntry, owner: str) -> CommandEntry:
    """Return a command entry marked with the plugin that registered it."""
    return CommandEntry(entry.name, entry.description, entry.permission, entry.handler,
                        owner=owner)


def _owned_skill(entry: SkillEntry, owner: str) -> SkillEntry:
    """Return a skill entry marked with the plugin that shipped it."""
    return SkillEntry(entry.name, entry.description, entry.body, owner=owner)


# ---------------------------------------------------------------- the files


def edit_config(path: pathlib.Path, edit: Callable[[str], str], *, now: datetime) -> bool:
    """Back up <path> to <path>.bak-<UTC stamp>, apply a text edit, keep every other byte.

    Returns whether the file changed. The backup carries the clock's time, never the wall
    clock, so a replay writes the same names.
    """
    path = pathlib.Path(path)
    text = path.read_text(encoding="utf-8")
    changed = edit(text)
    if changed == text:
        return False
    stamp = now.astimezone(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    shutil.copy2(path, path.with_name(f"{path.name}.bak-{stamp}"))
    path.write_text(changed, encoding="utf-8")
    return True


PLUGINS_BLOCK = re.compile(r"^plugins:(?:\s+#.*)?\s*$")


def set_plugin_enabled(text: str, name: str, enabled: bool) -> str:
    """Return the text with only `plugins.<name>.enabled` flipped; every other byte stays."""
    value = "true" if enabled else "false"
    lines = text.splitlines(keepends=True)
    start = next((index for index, line in enumerate(lines)
                  if PLUGINS_BLOCK.match(line.rstrip("\n"))), None)
    entry = re.compile(rf"^(\s+){re.escape(name)}:\s*(.*)$")
    if start is None:
        block = f"plugins:\n  {name}: {{enabled: {value}}}\n"
        tail = "" if not text or text.endswith("\n") else "\n"
        return f"{text}{tail}{block}"
    end = start + 1
    while end < len(lines) and (lines[end].startswith((" ", "\t")) or not lines[end].strip()):
        end += 1
    for index in range(start + 1, end):
        match = entry.match(lines[index].rstrip("\n"))
        if match:
            indent, rest = match.group(1), match.group(2)
            if re.search(r"enabled:\s*(true|false)", rest):
                rest = re.sub(r"enabled:\s*(true|false)", f"enabled: {value}", rest, count=1)
            elif rest.startswith("{"):
                rest = "{" + f"enabled: {value}" + (", " + rest[1:] if rest[1:].strip(" }") else "")
            else:
                rest = f"{{enabled: {value}}}"
            lines[index] = f"{indent}{name}: {rest}\n"
            return "".join(lines)
    insert_at = end
    while insert_at > start + 1 and not lines[insert_at - 1].strip():
        insert_at -= 1
    lines.insert(insert_at, f"  {name}: {{enabled: {value}}}\n")
    return "".join(lines)


def set_person_roles(text: str, who: str, held: Sequence[str]) -> str:
    """Return the text with only `people.<id>.roles` set; every other byte stays."""
    key = json.dumps(who)  # the id holds colons, so it is written as a quoted YAML key
    rendered = "[" + ", ".join(str(role) for role in held) + "]"
    lines = text.splitlines(keepends=True)
    head = re.compile(r"^people:\s*(\{\s*\})?(?:\s+#.*)?\s*$")
    start = next((index for index, line in enumerate(lines)
                  if head.match(line.rstrip("\n"))), None)
    if start is None:
        tail = "" if not text or text.endswith("\n") else "\n"
        return f"{text}{tail}people:\n  {key}: {{roles: {rendered}}}\n"
    if lines[start].rstrip("\n").split("#")[0].strip() != "people:":
        lines[start] = "people:\n"  # `people: {}` becomes a block with this one entry
    end = start + 1
    while end < len(lines) and (lines[end].startswith((" ", "\t")) or not lines[end].strip()):
        end += 1
    entry = re.compile(rf"^(\s+)(?:{re.escape(key)}|'{re.escape(who)}'|"
                       rf"{re.escape(who)}):\s*(.*)$")
    for index in range(start + 1, end):
        match = entry.match(lines[index].rstrip("\n"))
        if match is None:
            continue
        rest, indent = match.group(2), match.group(1)
        if "roles:" in rest:
            lines[index] = (f"{indent}{key}: "
                            + re.sub(r"roles:\s*\[[^\]]*\]", f"roles: {rendered}", rest,
                                     count=1) + "\n")
            return "".join(lines)
        if rest.strip() in ("", "{}"):
            for deeper in range(index + 1, end):  # a block-style entry
                if re.match(rf"^{indent}\s+roles:", lines[deeper]):
                    pad = re.match(r"^(\s+)", lines[deeper]).group(1)
                    lines[deeper] = f"{pad}roles: {rendered}\n"
                    return "".join(lines)
    insert_at = end
    while insert_at > start + 1 and not lines[insert_at - 1].strip():
        insert_at -= 1
    lines.insert(insert_at, f"  {key}: {{roles: {rendered}}}\n")
    return "".join(lines)


def set_block_entry(text: str, block: str, name: str, value: Any) -> str:
    """Return the text with one inline map entry set under a top-level block, nothing else."""
    rendered = json.dumps(value) if not isinstance(value, str) else value
    lines = text.splitlines(keepends=True)
    head = re.compile(rf"^{re.escape(block)}:(?:\s+#.*)?\s*$")
    start = next((index for index, line in enumerate(lines)
                  if head.match(line.rstrip("\n"))), None)
    if start is None:
        return f"{text}{'' if text.endswith(chr(10)) or not text else chr(10)}{block}:\n  {name}: {rendered}\n"
    end = start + 1
    while end < len(lines) and (lines[end].startswith((" ", "\t")) or not lines[end].strip()):
        end += 1
    entry = re.compile(rf"^(\s+){re.escape(name)}:\s*(.*)$")
    for index in range(start + 1, end):
        match = entry.match(lines[index].rstrip("\n"))
        if match:
            lines[index] = f"{match.group(1)}{name}: {rendered}\n"
            return "".join(lines)
    lines.insert(end, f"  {name}: {rendered}\n")
    return "".join(lines)


async def record_config_change(kernel: Any, event: Any, *, plugin: str, key: str,
                               before: Any, after: Any) -> None:
    """Record one configuration change as history: who, from what, to what (D59)."""
    await append_event(kernel.conn, ts=kernel.clock.now(), kind="plugin.config_changed",
                       who=event.origin_actor or event.who, place=event.place,
                       body=event.body, caused_by=event.id,
                       payload={"plugin": plugin, "key": key, "from": before, "to": after})


# ------------------------------------------------------------- the commands

USAGE = "usage: /plugin show|enable|disable|update <name>"
DEPLOYMENT_KEYS = ("plugins", "bodies", "roles", "people", "places", "interaction",
                   "owner_settings", "timezone", "runtime")


def install_commands(loader: "Loader", kernel: Any) -> None:
    """Register the loader's own commands: `/plugin` and `/reload` (middleware.md §5)."""

    async def plugin(kernel: Any, event: Any, arguments: list[str]) -> str:
        """Show, enable, disable or update one plugin; the files are the source of truth."""
        action = arguments[0].lower() if arguments else "show"
        name = arguments[1] if len(arguments) > 1 else ""
        if not name:
            return USAGE
        if action == "show":
            return _defaults(loader, name)
        if action in ("enable", "disable"):
            return await _toggle(loader, kernel, event, name, action == "enable")
        if action == "update":
            return await _update(loader, kernel, event, name)
        return USAGE

    async def reload(kernel: Any, event: Any, arguments: list[str]) -> str:
        """Re-read the files and apply them in place; on any error keep the running one."""
        return (await reload_into(loader, kernel, event))[0]

    if kernel.tools.command("plugin") is None:
        kernel.tools.add_command(CommandEntry(
            "plugin", "Show, enable, disable or update a plugin.", "plugin.manage", plugin,
            owner="loader"))
    if kernel.tools.command("reload") is None:
        kernel.tools.add_command(CommandEntry(
            "reload", "Re-read config.yaml, config.d/ and the manifests.", "config.reload",
            reload, owner="loader"))


async def reload_into(loader: "Loader", kernel: Any, event: Any) -> tuple[str, Loaded | None]:
    """Run the loader again and install it; return the reply and what was applied."""
    from monoagens.infra.prior import load_settings  # local: prior -> memory -> core

    notes: list[str] = []
    try:
        fresh = load_settings(loader.root / "config.yaml", loader.root / "soul.yaml",
                              loader.root / "prior", notes=notes)
        loaded = loader.load(fresh, notes=notes, who=event.origin_actor or event.who)
    except Exception as exc:
        return f"reload refused: {exc}", None
    restarts = [f"restart needed: plugin {api.name} "
                f"{getattr(kernel, 'plugin_versions', {}).get(api.name)} -> "
                f"{api.manifest.get('version')}"
                for api in loaded.apis
                if getattr(kernel, "plugin_versions", {}).get(api.name)
                not in (None, str(api.manifest.get("version") or ""))]
    await loader.install(kernel, loaded)
    for key in DEPLOYMENT_KEYS:
        if key in fresh:
            kernel.settings[key] = fresh[key]
    reply = (f"reload ok: {len(loaded.plugins)} plugins, {len(kernel.tools.entries)} tools")
    return "\n".join([reply, *restarts]), loaded


def _defaults(loader: "Loader", name: str) -> str:
    """Render one plugin's defaults as YAML: settings, permissions and role settings."""
    manifest = loader._manifest(name)
    defaults = {
        "settings": {key: spec.get("default")
                     for key, spec in (manifest.get("settings") or {}).items()},
        "permissions": {key: (spec or {}).get("base")
                        for key, spec in (manifest.get("permissions") or {}).items()},
        "role_settings": manifest.get("role_settings") or {},
    }
    return f"# {name} {manifest.get('version')}\n" + yaml.safe_dump(
        defaults, sort_keys=True, allow_unicode=True)


def _write_config_d(loader: "Loader", name: str, manifest: Mapping[str, Any],
                    now: datetime) -> bool:
    """Create config.d/<name>.yaml from the manifest the first time; keep the owner's file."""
    path = loader.root / "config.d" / f"{name}.yaml"
    if path.is_file():
        return False
    values = {
        "version": CONFIG_D_VERSION,
        "permissions": {key: (spec or {}).get("base")
                        for key, spec in (manifest.get("permissions") or {}).items()},
        "role_settings": manifest.get("role_settings") or {},
    }
    text = (f"# written by /plugin enable from {name} {manifest.get('version')}\n"
            + yaml.safe_dump(values, sort_keys=True, allow_unicode=True))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return True


async def _toggle(loader: "Loader", kernel: Any, event: Any, name: str, enabled: bool) -> str:
    """Enable or disable one plugin: one line in config.yaml, then the same reload."""
    manifest = loader._manifest(name) if enabled else None
    path = loader.root / "config.yaml"
    changed = edit_config(path, lambda text: set_plugin_enabled(text, name, enabled),
                          now=kernel.clock.now())
    if changed:
        await record_config_change(kernel, event, plugin=name,
                                   key=f"config.yaml:plugins.{name}.enabled",
                                   before=not enabled, after=enabled)
    if manifest is not None and _write_config_d(loader, name, manifest, kernel.clock.now()):
        await record_config_change(kernel, event, plugin=name,
                                   key=f"config.d/{name}.yaml", before=None,
                                   after={"version": CONFIG_D_VERSION})
    reply, _ = await reload_into(loader, kernel, event)
    return f"/plugin {'enable' if enabled else 'disable'} {name}: {reply}"


async def _update(loader: "Loader", kernel: Any, event: Any, name: str) -> str:
    """The dpkg rule: move only the lines still equal to what `/plugin enable` wrote."""
    manifest = loader._manifest(name)
    path = loader.root / "config.d" / f"{name}.yaml"
    if not path.is_file():
        return f"/plugin update {name}: no config.d/{name}.yaml"
    current = config_module.read_versioned(path, key="version", supported=CONFIG_D_VERSION,
                                           label=f"config.d/{name}.yaml", upcasters={})
    defaults = {
        "permissions": {key: (spec or {}).get("base")
                        for key, spec in (manifest.get("permissions") or {}).items()},
        "role_settings": manifest.get("role_settings") or {},
        "options": {key: spec.get("default")
                    for key, spec in (manifest.get("settings") or {}).items()},
    }
    recorded = await _recorded_defaults(kernel, name)
    kept, moved = {}, []
    for section, entries in defaults.items():
        for key, default in entries.items():
            present = (current.get(section) or {}).get(key, _MISSING)
            if present is _MISSING or present == default:
                continue
            if recorded.get(f"{section}.{key}") == present:
                kept.setdefault(section, {})[key] = default
                moved.append(f"{section}.{key}")
            else:
                moved.append(f"{section}.{key} kept (changed by the owner)")
    if not moved:
        return f"/plugin update {name}: nothing to update"
    if kept:
        updated = {key: dict(value or {}) for key, value in current.items()
                   if key in CONFIG_D_KEYS}
        for section, entries in kept.items():
            updated[section] = {**(updated.get(section) or {}), **entries}
        text = yaml.safe_dump(updated, sort_keys=True, allow_unicode=True)
        edit_config(path, lambda _old: text, now=kernel.clock.now())
        for section, entries in kept.items():
            for key, value in entries.items():
                await record_config_change(kernel, event, plugin=name,
                                           key=f"config.d/{name}.yaml:{section}.{key}",
                                           before=current.get(section, {}).get(key),
                                           after=value)
    return f"/plugin update {name}: " + ", ".join(moved)


_MISSING = object()


async def _recorded_defaults(kernel: Any, name: str) -> dict[str, Any]:
    """Return the values `/plugin enable` wrote, the latest per key, from the Log."""
    cur = await kernel.conn.execute(
        "SELECT payload FROM events WHERE kind = 'plugin.config_changed' "
        "AND payload->>'plugin' = %s AND payload->>'key' LIKE %s ORDER BY id",
        (name, f"config.d/{name}.yaml:%"))
    found: dict[str, Any] = {}
    for (payload,) in await cur.fetchall():
        found[str(payload["key"]).split(":", 1)[1]] = payload.get("to")
    return found

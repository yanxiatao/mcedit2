"""Python 3 plugin discovery, registration and lifecycle for MCEdit2 Next.

Plugins execute trusted in-process Python. Discovery never imports code; loading
requires an explicit user action. Plugins can access the host OS and are NOT
sandboxed. Editing callbacks receive an undo-aware transaction interface.
"""
from __future__ import annotations

from contextvars import ContextVar
from dataclasses import dataclass, field
import importlib.util
import hashlib
import re
import sys
from pathlib import Path
from typing import Any, Callable
import tomllib

from .core import Box, EditSession, EditTransaction
from . import nbtwire

PLUGIN_API = 1
IDENTIFIER = re.compile(r"^[a-z][a-z0-9_.-]{2,79}$")
BLOCK_ENTITY_ID = re.compile(r"^[a-z0-9_.-]+:[a-z0-9_./-]+$")
EXTENSION_TYPES = frozenset({
    "command", "tool", "brush_mode", "brush_shape", "inspector",
    "tile_entity", "generate", "widget", "filter",
})


class PluginError(RuntimeError):
    pass


@dataclass(frozen=True)
class Manifest:
    id: str
    name: str
    version: str
    root: Path
    entrypoint: str = "__init__.py"
    api: int = PLUGIN_API
    description: str = ""


@dataclass(frozen=True)
class Command:
    owner: str
    id: str
    label: str
    callback: Callable[..., Any]
    options: tuple[dict[str, Any], ...] = ()
    submenu: str | None = None
    legacy: bool = False


@dataclass(frozen=True)
class BlockInspection:
    """Snapshot of one block for explicitly loaded inspector plugins."""
    position: tuple[int, int, int]
    state: Any
    block_entity: bytes | None

@dataclass(frozen=True)
class FilterDefinition:
    """A typed options schema for a native Python 3 predicate."""
    owner: str
    id: str
    label: str
    callback: Callable[..., Any]
    options: tuple[dict[str, Any], ...] = ()


@dataclass(frozen=True)
class FilterBlock:
    """Original-format block snapshot for an explicitly enabled filter."""
    position: tuple[int, int, int]
    state: Any
    has_block_entity: bool | None


@dataclass(frozen=True)
class FilterPreview:
    """Read-only count; protected_be=None means the backend cannot check."""
    total_blocks: int
    matches: int
    changes: int
    protected_be: int | None


@dataclass
class PluginState:
    manifest: Manifest
    loaded: bool = False
    error: str | None = None
    module_name: str | None = None


def read_manifest(root: Path) -> Manifest:
    root = Path(root).resolve()
    manifest_path = root / "plugin.toml"
    if not manifest_path.is_file():
        raise PluginError("Missing plugin.toml")
    try:
        document = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        config = document["plugin"]
        identifier = config["id"]
        title = config["name"]
        version = config["version"]
        api = config.get("api", 1)
        entry = config.get("entrypoint", "__init__.py")
        description = config.get("description", "")
    except (KeyError, ValueError, OSError, TypeError, tomllib.TOMLDecodeError) as exc:
        raise PluginError("Malformed plugin.toml") from exc
    if not isinstance(identifier, str) or not IDENTIFIER.fullmatch(identifier):
        raise PluginError("Invalid plugin id: use lowercase letters, digits, '.', '-', '_'")
    if not all(isinstance(v, str) and v for v in (title, version, entry)):
        raise PluginError("Plugin name, version and entrypoint must be nonempty strings")
    if not isinstance(description, str):
        raise PluginError("Plugin description must be a string")
    if type(api) is not int or api != PLUGIN_API:
        raise PluginError(f"Unsupported plugin API {api!r}; this editor supports API {PLUGIN_API}")
    # Only one Python file under the package root may be executed as entrypoint.
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*\.py", entry):
        raise PluginError("Unsafe entrypoint; expected __init__.py or a simple .py filename")
    path = root / entry
    if not path.is_file() or path.is_symlink() or not path.resolve().is_relative_to(root):
        raise PluginError("Entrypoint absent or outside plugin folder")
    return Manifest(identifier, title, version, root, entry, api, description)


def _check_options(options):
    if not isinstance(options, (tuple, list)):
        raise PluginError("Plugin command options must be a list")
    allowed = {"int", "float", "bool", "text", "choice", "blocktype", "label"}
    names = set()
    normalized = []
    for option in options:
        if not isinstance(option, dict) or option.get("type") not in allowed:
            raise PluginError("Unsupported plugin option type")
        option = dict(option)
        if option["type"] != "label":
            name = option.get("name")
            if not isinstance(name, str) or not name or name in names:
                raise PluginError("Missing or duplicate plugin option name")
            names.add(name)
        if option["type"] == "choice" and not isinstance(option.get("choices"), (list, tuple)):
            raise PluginError("Choice options require a list of choices")
        normalized.append(option)
    return tuple(normalized)


class PluginHost:
    def __init__(self, plugin_id: str):
        self.plugin_id = plugin_id
        self.commands: dict[str, Command] = {}
        self.extensions: dict[str, dict[str, Any]] = {key: {} for key in EXTENSION_TYPES - {"command"}}
        self.tile_entity_decoders: dict[str, Callable[[BlockInspection], Any]] = {}
        self.filter_definitions: dict[str, FilterDefinition] = {}

    def register_command(self, identifier: str, label: str, callback: Callable[..., Any],
                         options=(), submenu: str | None = None, legacy: bool = False):
        if not isinstance(identifier, str) or not IDENTIFIER.fullmatch(identifier):
            raise PluginError("Invalid command ID")
        if identifier in self.commands or not callable(callback):
            raise PluginError("Duplicate command or invalid callback")
        if not isinstance(label, str) or not label.strip():
            raise PluginError("Command label must be nonempty")
        command = Command(self.plugin_id, identifier, label, callback,
                          _check_options(options), submenu, legacy)
        self.commands[identifier] = command
        return callback

    def register_extension(self, category: str, identifier: str, provider: Any):
        if category not in self.extensions:
            raise PluginError("Unknown extension category")
        if not isinstance(identifier, str) or not IDENTIFIER.fullmatch(identifier):
            raise PluginError("Invalid extension ID")
        if identifier in self.extensions[category] or provider is None:
            raise PluginError("Duplicate or invalid extension")
        self.extensions[category][identifier] = provider
        return provider

    def register_filter(self, identifier: str, label: str, callback, options=()):
        """Opt-in typed native filter. Legacy single-argument filters still work."""
        if not isinstance(label, str) or not label.strip():
            raise PluginError("Filter label must be nonempty")
        if not callable(callback) or isinstance(callback, type):
            raise PluginError("Filter predicate must be a Python 3 callable")
        schema = _check_options(options)
        if any(field.get("name") == "__mcedit2_destination" for field in schema):
            raise PluginError("Reserved filter option name")
        self.register_extension("filter", identifier, callback)
        definition = FilterDefinition(
            self.plugin_id, identifier, label.strip(), callback, schema)
        self.filter_definitions[identifier] = definition
        return callback

    def register_tile_entity_decoder(self, block_entity_id: str, callback):
        """Register an exact NBT id decoder, separate from legacy reference classes."""
        if (not isinstance(block_entity_id, str)
                or not BLOCK_ENTITY_ID.fullmatch(block_entity_id)
                or any(part in ("", ".", "..") for part in
                       block_entity_id.partition(":")[2].split("/"))):
            raise PluginError("Decoder needs a namespaced block entity ID")
        if (block_entity_id in self.tile_entity_decoders
                or not callable(callback) or isinstance(callback, type)):
            raise PluginError("Duplicate decoder or non-callable Python 3 decoder")
        self.tile_entity_decoders[block_entity_id] = callback
        return callback


_loading_host: ContextVar[PluginHost | None] = ContextVar("mcedit_plugin_host", default=None)


class SimpleCommandPlugin:
    """Migration base for MCEdit2's SimpleCommandPlugin.perform interface.

    Port imports to mcedit2_next.plugins and remove PySide 1 imports.
    perform(dimension, selection, options) remains the same basic signature.
    """

    displayName = None
    submenuName = None
    options = []

    def perform(self, dimension, selection, options):
        raise NotImplementedError


def registerPluginCommand(cls):
    """Legacy-style class decorator, active only while a manifest plugin loads."""
    host = _loading_host.get()
    if host is None:
        raise PluginError("Plugin command registration is only valid during plugin load")
    if not isinstance(cls, type) or not issubclass(cls, SimpleCommandPlugin):
        raise PluginError("Expected a SimpleCommandPlugin subclass")
    label = getattr(cls, "displayName", None)
    identifier = re.sub(r"(?<!^)(?=[A-Z])", "_", cls.__name__).lower()
    identifier = re.sub("[^a-z0-9_.-]", "_", identifier)
    if len(identifier) < 3:
        identifier = "cmd_" + identifier
    def run(ctx, options):
        instance = cls()
        return instance.perform(LegacyDimension(ctx), LegacySelection(ctx), options)
    host.register_command(identifier, label, run, cls.options,
                          getattr(cls, "submenuName", None), legacy=True)
    return cls


def _legacy_extension(category: str, cls: type, suffix: str = ""):
    host = _loading_host.get()
    if host is None:
        raise PluginError("Plugin registration requires a loading plugin module")
    if not isinstance(cls, type):
        raise PluginError("Legacy extension registration expects a class")
    identifier = re.sub(r"(?<!^)(?=[A-Z])", "_", cls.__name__).lower()
    identifier = re.sub("[^a-z0-9_.-]", "_", identifier)
    if len(identifier) < 3:
        identifier = "ext_" + identifier
    if suffix:
        identifier += "_" + hashlib.sha256(suffix.encode("utf-8")).hexdigest()[:10]
    host.register_extension(category, identifier, cls)
    return cls


def registerToolClass(cls):
    """Preserve registration from MCEdit2; Qt6 tool binding is not ported."""
    return _legacy_extension("tool", cls)


def registerGeneratePlugin(cls):
    """Preserve generator registration; UI binding is not ported."""
    return _legacy_extension("generate", cls)


def registerCustomWidget(cls):
    """Preserve widget registration; PySide1 .ui integration is not ported."""
    return _legacy_extension("widget", cls)


def registerBlockInspectorWidget(cls):
    """Preserve inspector registration; NBT editing UI is not ported."""
    return _legacy_extension("inspector", cls)


def registerTileEntityRefClass(identifier, cls):
    """Preserve the legacy BE type association without parsing or writing it."""
    if not isinstance(identifier, str) or not identifier:
        raise PluginError("Block entity type ID must be a nonempty string")
    return _legacy_extension("tile_entity", cls, identifier)


class LegacyBlockTypes:
    def __init__(self, ctx):
        self.ctx = ctx

    def __getitem__(self, name):
        return self.ctx.make_block(name)


class LegacyDimension:
    def __init__(self, context):
        self.context = context
        self.blocktypes = LegacyBlockTypes(context)

    def getBlock(self, x, y, z):
        return self.context.get_block(x, y, z)

    def setBlock(self, x, y, z, block):
        return self.context.set_block(x, y, z, block)

    def getEntities(self, *args, **kwargs):
        raise NotImplementedError("Entity access requires the future NBT plugin API")


class LegacySelection:
    def __init__(self, context):
        self.context = context

    @property
    def positions(self):
        if self.context.selection is None:
            raise PluginError("This command requires a selected region")
        self.context.transaction.session._check_size(self.context.selection.volume)
        return self.context.selection.points()


class PluginContext:
    """Narrow cooperative editing API, not a process security boundary."""

    def __init__(self, transaction: EditTransaction, selection: Box | None):
        self.transaction = transaction
        self.selection = selection

    def get_block(self, x: int, y: int, z: int):
        return self.transaction.get_block((x, y, z))

    def set_block(self, x: int, y: int, z: int, block: Any):
        return self.transaction.set_block((x, y, z), block)

    def make_block(self, identifier: str):
        return self.transaction.session.world.make_block(identifier)

    def get_block_entity_raw(self, x: int, y: int, z: int) -> bytes | None:
        """Get immutable BE NBT bytes. Writing or normalizing is forbidden."""
        getter = getattr(self.transaction.session.world, "get_block_entity_raw", None)
        if not callable(getter):
            raise PluginError("The current world backend does not expose BE inspection")
        return getter(x, y, z)

    def fill(self, box: Box, block):
        self.transaction.session._check_size(box.volume)
        for point in box.points():
            self.transaction.set_block(point, block)

    def sphere(self, center, radius, block, hollow=False):
        """Expose undoable geometry through the plugin's staged edit context."""
        if not 1 <= radius <= 64:
            raise PluginError("Plugin sphere radius must be 1..64")
        outer = radius * radius
        inner = (radius - 1) * (radius - 1)
        for dy in range(-radius, radius + 1):
            for dz in range(-radius, radius + 1):
                for dx in range(-radius, radius + 1):
                    d2 = dx*dx + dy*dy + dz*dz
                    if d2 <= outer and (not hollow or d2 >= inner):
                        self.transaction.set_block(
                            (center[0] + dx, center[1] + dy, center[2] + dz),
                            block)

    def replace(self, box: Box, before, after):
        self.transaction.session._check_size(box.volume)
        for point in box.points():
            if self.transaction.get_block(point) == before:
                self.transaction.set_block(point, after)


class PluginManager:
    def __init__(self, directories):
        self.directories = tuple(Path(path).expanduser().resolve() for path in directories)
        self.available: dict[str, PluginState] = {}
        self.hosts: dict[str, PluginHost] = {}
        self.modules: dict[str, Any] = {}

    def discover(self) -> dict[str, PluginState]:
        found = {}
        for directory in self.directories:
            if not directory.is_dir():
                continue
            for folder in sorted(directory.iterdir()):
                if not folder.is_dir() or folder.is_symlink() or not (folder / "plugin.toml").is_file():
                    continue
                try:
                    manifest = read_manifest(folder)
                except PluginError:
                    continue
                if manifest.id in found:
                    continue
                state = self.available.get(manifest.id)
                if state is None or state.manifest.root != manifest.root:
                    state = PluginState(manifest)
                found[manifest.id] = state
        self.available = {**{key: state for key, state in self.available.items()
                              if state.loaded}, **found}
        return dict(self.available)

    def load(self, identifier: str):
        state = self.available.get(identifier)
        if state is None:
            raise PluginError("Unknown plugin; rescan the plugin directories")
        if state.loaded:
            return
        manifest = read_manifest(state.manifest.root)
        if manifest.id != identifier:
            raise PluginError("Plugin manifest ID changed")
        module_name = "_mcedit2_next_plugin_" + hashlib.sha256(
            identifier.encode("utf-8")).hexdigest()[:24]
        path = manifest.root / manifest.entrypoint
        host = PluginHost(identifier)
        if manifest.entrypoint == "__init__.py":
            spec = importlib.util.spec_from_file_location(
                module_name, path, submodule_search_locations=[str(manifest.root)])
        else:
            spec = importlib.util.spec_from_file_location(module_name, path)
        if spec is None or spec.loader is None:
            raise PluginError("Cannot load plugin Python module")
        module = importlib.util.module_from_spec(spec)
        state.error = None
        token = _loading_host.set(host)
        sys.modules[module_name] = module
        try:
            # Compile the current source to avoid stale timestamp-based .pyc on fast reload.
            exec(compile(path.read_bytes(), str(path), 'exec'), module.__dict__)
            if hasattr(module, "register"):
                module.register(host)
            state.loaded = True
            state.module_name = module_name
            self.hosts[identifier] = host
            self.modules[identifier] = module
        except Exception as exc:
            state.error = f"{type(exc).__name__}: {exc}"
            for name in tuple(sys.modules):
                if name == module_name or name.startswith(module_name + "."):
                    sys.modules.pop(name, None)
            raise PluginError(f"Loading {identifier}: {state.error}") from exc
        finally:
            _loading_host.reset(token)

    def unload(self, identifier: str):
        state = self.available.get(identifier)
        if state is None or not state.loaded:
            return
        module = self.modules.get(identifier)
        if module is not None and callable(getattr(module, "unregister", None)):
            module.unregister()
        self.hosts.pop(identifier, None)
        self.modules.pop(identifier, None)
        module_name = state.module_name
        state.loaded = False
        state.module_name = None
        for name in tuple(sys.modules):
            if name == module_name or name.startswith(module_name + "."):
                sys.modules.pop(name, None)

    def reload(self, identifier: str):
        self.unload(identifier)
        self.discover()
        self.load(identifier)

    def commands(self):
        return tuple(command for host in self.hosts.values()
                     for command in host.commands.values())

    def extensions(self, category: str):
        if category not in EXTENSION_TYPES - {"command"}:
            raise PluginError("Unknown extension category")
        return {f"{pid}:{key}": value for pid, host in self.hosts.items()
                for key, value in host.extensions[category].items()}

    def execute(self, command: Command, session: EditSession,
                selection: Box | None = None, options: dict | None = None):
        host = self.hosts.get(command.owner)
        if host is None or host.commands.get(command.id) is not command:
            raise PluginError("Command is no longer registered")
        transaction = EditTransaction(session)
        context = PluginContext(transaction, selection)
        arguments = dict(options or {})
        for option in command.options:
            key = option.get("name")
            if key and key not in arguments:
                arguments[key] = option.get("value")
            if key and option["type"] == "blocktype" and isinstance(arguments[key], str):
                arguments[key] = context.make_block(arguments[key])
        try:
            result = command.callback(context, arguments)
            if transaction.changes:
                transaction.commit()
            return result
        except Exception:
            transaction.discard()
            raise

    def runnable_filters(self):
        """Only native Python 3 predicate callbacks, never legacy classes."""
        return {
            name: callback for name, callback in self.extensions("filter").items()
            if callable(callback) and not isinstance(callback, type)
        }

    def filter_definition(self, qualified_name: str) -> FilterDefinition | None:
        """A loaded configurable filter; ordinary one-argument filters return None."""
        owner, sep, identifier = qualified_name.partition(":")
        if not sep or qualified_name not in self.runnable_filters():
            return None
        host = self.hosts.get(owner)
        if host is None:
            return None
        definition = host.filter_definitions.get(identifier)
        if definition is None or host.extensions["filter"].get(identifier) is not definition.callback:
            return None
        return definition

    def _filter_arguments(self, qualified_name, options, world):
        """Resolve and type-check dialog defaults before running a predicate."""
        definition = self.filter_definition(qualified_name)
        supplied = dict(options or {})
        if definition is None:
            if supplied:
                raise PluginError("This filter has no configurable options")
            return None
        fields = {field["name"]: field for field in definition.options
                  if field["type"] != "label"}
        if supplied.keys() - fields.keys():
            raise PluginError("Unknown filter option")
        arguments = {}
        for name, field in fields.items():
            kind = field["type"]
            value = supplied.get(name, field.get("value"))
            if kind == "bool":
                if type(value) is not bool:
                    raise PluginError(f"Filter option {name} must be bool")
            elif kind == "int":
                if type(value) is not int:
                    raise PluginError(f"Filter option {name} must be int")
                if not int(field.get("min", -2_000_000_000)) <= value <= int(
                        field.get("max", 2_000_000_000)):
                    raise PluginError(f"Filter option {name} is outside its range")
            elif kind == "float":
                from math import isfinite
                if type(value) not in (int, float) or not isfinite(value):
                    raise PluginError(f"Filter option {name} must be a finite number")
                if not float(field.get("min", -1e12)) <= value <= float(
                        field.get("max", 1e12)):
                    raise PluginError(f"Filter option {name} is outside its range")
                value = float(value)
            elif kind == "text":
                if type(value) is not str or len(value) > 1024:
                    raise PluginError(f"Filter option {name} must be text (max 1024)")
            elif kind == "choice":
                choices = field.get("choices", ())
                if not any(isinstance(item, (tuple, list)) and len(item) == 2
                           and type(item[1]) is type(value) and item[1] == value
                           for item in choices):
                    raise PluginError(f"Filter option {name} is not an allowed choice")
            elif kind == "blocktype":
                if type(value) is not str:
                    if not (hasattr(value, "name") and
                            hasattr(value, "properties")):
                        raise PluginError(f"Filter option {name} must be a block state")
                    # Preserve version-native representation of typed states.
                    value = str(value)
                value = world.make_block(value)
            else:
                raise PluginError(f"Unsupported filter option type: {kind}")
            arguments[name] = value
        return arguments

    def _scan_filter(self, qualified_name: str, session: EditSession,
                     selection: Box | None, target, options: dict | None = None):
        """Evaluate a trusted read-only predicate before staging changes.

        The callback signature is predicate(FilterBlock) -> bool. It must
        not write to the world, and receives no write-capable context.
        Loading plugins is trusted in-process execution, not a sandbox.
        """
        provider = self.runnable_filters().get(qualified_name)
        if provider is None:
            raise PluginError("Filter unloaded or not a Python 3 predicate")
        arguments = self._filter_arguments(qualified_name, options, session.world)
        if selection is None:
            raise PluginError("Filter requires a bounded selection")
        session._check_size(selection.volume)
        world = session.world
        checker = getattr(world, "has_block_entity", None)
        has_checker = callable(checker)
        conflicts = 0 if has_checker else None
        matches = 0
        updates = []
        for point in selection.points():
            state = world.get_block(*point)
            protected = bool(checker(*point)) if has_checker else None
            record = FilterBlock(point, state, protected)
            outcome = (provider(record) if arguments is None else
                       provider(record, dict(arguments)))
            if type(outcome) is not bool:
                raise PluginError(
                    "Filter predicate must return a bool for each block")
            if not outcome:
                continue
            matches += 1
            if state == target:
                continue
            updates.append((point, target))
            if conflicts is not None and protected:
                conflicts += 1
        return (FilterPreview(selection.volume, matches, len(updates), conflicts),
                updates)

    def preview_filter(self, qualified_name: str, session: EditSession,
                       selection: Box | None, target,
                       options: dict | None = None) -> FilterPreview:
        """Preview without changing the world, undo history or BE NBT."""
        preview, _ = self._scan_filter(
            qualified_name, session, selection, target, options)
        return preview

    def apply_filter(self, qualified_name: str, session: EditSession,
                     selection: Box | None, target,
                     options: dict | None = None) -> int:
        """Rescan after confirmation and commit one guarded undo operation."""
        preview, updates = self._scan_filter(
            qualified_name, session, selection, target, options)
        if not updates:
            return 0
        if preview.protected_be is None:
            raise PluginError(
                "Backend cannot check BE presence; filter write refused")
        if preview.protected_be:
            raise PluginError(
                f"{preview.protected_be} Block Entity conflicts; "
                "refusing entire filter operation")
        # Version-native preflight and rollback are owned by the editor core.
        return session._commit(updates)

    def available_tile_entity_decoders(self):
        """Qualified decoder -> (exact NBT id, callback) for enabled plugins."""
        return {
            f"{pid}:{block_id}": (block_id, callback)
            for pid, host in self.hosts.items()
            for block_id, callback in host.tile_entity_decoders.items()
        }

    @staticmethod
    def _format_inspection(result):
        if isinstance(result, str):
            output = result
        elif isinstance(result, dict):
            if len(result) > 100 or any(
                    not isinstance(key, str)
                    or type(value) not in (str, int, float, bool, type(None))
                    for key, value in result.items()):
                raise PluginError("Inspector must return up to 100 scalar fields")
            output = "\n".join(f"{key}: {value}" for key, value in result.items())
        else:
            raise PluginError("Inspector must return text or a scalar dictionary")
        if len(output) > 16000:
            raise PluginError("Inspector output exceeds display length limit")
        return output

    def inspect_tile_entity(self, qualified_name: str, world, point):
        """Invoke a decoder only on a matching original block entity NBT id."""
        if (not isinstance(point, (tuple, list)) or len(point) != 3
                or any(type(v) is not int for v in point)):
            raise PluginError("Decoder point must have three integer coordinates")
        entry = self.available_tile_entity_decoders().get(qualified_name)
        if entry is None:
            raise PluginError("BE decoder unloaded or unavailable")
        block_id, callback = entry
        getter = getattr(world, "get_block_entity_raw", None)
        if not callable(getter):
            raise PluginError("World does not expose original BE NBT")
        payload = getter(*point)
        if payload is None:
            raise PluginError("No block entity exists at this position")
        if not isinstance(payload, bytes):
            raise PluginError("Unsupported BE payload type")
        try:
            root = nbtwire.parse(nbtwire.make_tag(10, "", payload))
            actual_id = root.require("id", 8).value
        except (ValueError, TypeError) as exc:
            raise PluginError("Missing or malformed original BE NBT id") from exc
        if actual_id != block_id:
            raise PluginError(
                f"Decoder expects {block_id}, but original BE id is {actual_id}")
        record = BlockInspection(tuple(point), world.get_block(*point), payload)
        return self._format_inspection(callback(record))

    def inspect_block(self, qualified_name: str, world, point):
        """Invoke a trusted Python 3 read-only inspector callback.

        Callback accepts BlockInspection and returns text or a dictionary of
        human-readable scalar fields. Legacy PySide1 inspector widget classes
        are deliberately never instantiated. Python plugins remain trusted
        in-process code, not a security sandbox.
        """
        if (not isinstance(point, (tuple, list)) or len(point) != 3
                or any(type(v) is not int for v in point)):
            raise PluginError("Inspector point must have three integer coordinates")
        inspectors = {
            key: callback for key, callback in self.extensions("inspector").items()
            if callable(callback) and not isinstance(callback, type)
        }
        callback = inspectors.get(qualified_name)
        if callback is None:
            raise PluginError("Inspector unloaded or requires a Python 3 port")
        getter = getattr(world, "get_block_entity_raw", None)
        payload = getter(*point) if callable(getter) else None
        if payload is not None and not isinstance(payload, bytes):
            raise PluginError("Unsupported BE payload type")
        record = BlockInspection(
            tuple(point), world.get_block(*point), payload)
        result = callback(record)
        return self._format_inspection(result)

    def runnable_extensions(self, category: str):
        """Only explicit Python3 callback tools, not legacy PySide1 classes.

        Legacy tool/generator class registration is retained for migration,
        but it is unsafe to guess its constructor and old schematic API.
        """
        if category not in ("tool", "generate"):
            raise PluginError("Only tool and generate callbacks can be executed")
        return {key: callback for key, callback in self.extensions(category).items()
                if callable(callback) and not isinstance(callback, type)}

    def execute_native_extension(self, category: str, qualified_name: str,
                                 session: EditSession, selection: Box | None = None,
                                 options: dict | None = None):
        """Run a trusted Python 3 editor tool or generator transactionally.

        The callback signature is callback(context, options). It may use
        context.selection, context.set_block, context.fill etc. Old Python2
        / PySide1 plugin classes do not satisfy this contract.
        """
        runnable = self.runnable_extensions(category)
        provider = runnable.get(qualified_name)
        if provider is None:
            raise PluginError(
                "Tool unavailable, unloaded, or a legacy class requiring "
                "explicit Python 3 / Qt6 migration")
        if category == "generate" and selection is None:
            raise PluginError("Generator requires a bounded selected area")
        if selection is not None:
            session._check_size(selection.volume)
        transaction = EditTransaction(session)
        ctx = PluginContext(transaction, selection)
        try:
            result = provider(ctx, dict(options or {}))
            if transaction.changes:
                transaction.commit()
            return result
        except Exception:
            transaction.discard()
            raise

    def execute_brush_shape(self, qualified_name: str,
                            transaction: EditTransaction,
                            point: tuple[int, int, int], block: Any,
                            selection: Box | None = None):
        """Stage a Python3 brush shape as one part of a mouse stroke.

        Contract: provider(context, origin, native_block). Shape provider
        must use ctx.set_block and never bypass the transaction. Registered
        legacy PySide1 brush shape classes cannot execute automatically.
        """
        shapes = self.extensions("brush_shape")
        provider = shapes.get(qualified_name)
        if provider is None or not callable(provider) or isinstance(provider, type):
            raise PluginError("Brush shape unavailable or requires explicit Qt6 port")
        if len(point) != 3 or any(type(x) is not int for x in point):
            raise PluginError("Brush shape origin must have integer XYZ")
        return provider(PluginContext(transaction, selection), tuple(point), block)

    def execute_brush(self, qualified_name: str, transaction: EditTransaction,
                      point: tuple[int, int, int], selection: Box | None = None):
        """Apply one plugin brush stamp to the caller's staged stroke.

        The GUI owns commit/discard on mouse-release, so continuous dragging
        produces one undo record rather than hundreds of small records.
        """
        if ":" not in qualified_name:
            raise PluginError("Invalid brush identifier")
        plugin_id, name = qualified_name.split(":", 1)
        host = self.hosts.get(plugin_id)
        if host is None:
            raise PluginError("Brush plugin is unloaded")
        brush = host.extensions["brush_mode"].get(name)
        if not callable(brush):
            raise PluginError("Brush mode unavailable")
        return brush(PluginContext(transaction, selection), point)

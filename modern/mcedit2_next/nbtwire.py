"""Bounded, lossless NBT scanner with byte-span patching.

Unknown tags are preserved byte-for-byte. Only explicitly selected tags are
rewritten. This is intentionally independent of Amulet and nbtlib.
"""
from __future__ import annotations
from dataclasses import dataclass
import struct

MAX_NBT_BYTES = 64 * 1024 * 1024
MAX_DEPTH = 96
MAX_COLLECTION_ITEMS = 2_000_000


class NBTError(ValueError):
    pass


@dataclass(frozen=True)
class Tag:
    kind: int
    name: str
    start: int
    payload_start: int
    end: int
    value: object = None
    children: tuple["Tag", ...] = ()

    def child(self, name: str) -> "Tag | None":
        for item in self.children:
            if item.name == name:
                return item
        return None

    def require(self, name: str, kind: int | None = None) -> "Tag":
        item = self.child(name)
        if item is None or (kind is not None and item.kind != kind):
            raise NBTError(f"Missing or incompatible NBT tag {name!r}")
        return item


class Scanner:
    def __init__(self, data: bytes):
        if len(data) > MAX_NBT_BYTES:
            raise NBTError("NBT file exceeds size limit")
        self.data = data
        self.p = 0

    def consume(self, count: int) -> bytes:
        if count < 0 or self.p + count > len(self.data):
            raise NBTError("Truncated or invalid NBT payload")
        start = self.p
        self.p += count
        return self.data[start:self.p]

    def number(self, fmt: str):
        return struct.unpack(fmt, self.consume(struct.calcsize(fmt)))[0]

    def string(self) -> str:
        length = self.number(">H")
        try:
            return self.consume(length).decode("utf-8")
        except UnicodeDecodeError as exc:
            raise NBTError("Invalid NBT string") from exc

    def payload(self, kind: int, name: str, start: int, depth: int) -> Tag:
        if depth > MAX_DEPTH:
            raise NBTError("Excessive nested NBT depth")
        payload_start = self.p
        value = None
        children = ()
        if kind in (1, 2, 3, 4, 5, 6):
            fmt = {1: ">b", 2: ">h", 3: ">i", 4: ">q", 5: ">f", 6: ">d"}[kind]
            value = self.number(fmt)
        elif kind == 7:
            self.consume(self.number(">i"))
        elif kind == 8:
            value = self.string()
        elif kind == 9:
            child_kind = self.number(">B")
            length = self.number(">i")
            if length < 0 or length > MAX_COLLECTION_ITEMS or not (
                1 <= child_kind <= 12 or (length == 0 and child_kind == 0)
            ):
                raise NBTError("Invalid NBT list")
            children = tuple(self.payload(child_kind, "", self.p, depth + 1) for _ in range(length))
        elif kind == 10:
            result, seen = [], set()
            while True:
                child_type = self.number(">B")
                if child_type == 0:
                    break
                if not 1 <= child_type <= 12:
                    raise NBTError("Unsupported NBT tag type")
                child_start = self.p - 1
                child_name = self.string()
                if child_name in seen:
                    raise NBTError("Duplicate NBT compound tag")
                seen.add(child_name)
                result.append(self.payload(child_type, child_name, child_start, depth + 1))
                if len(result) > MAX_COLLECTION_ITEMS:
                    raise NBTError("NBT compound too large")
            children = tuple(result)
        elif kind in (11, 12):
            length = self.number(">i")
            if length < 0 or length > MAX_COLLECTION_ITEMS:
                raise NBTError("Invalid NBT array size")
            self.consume(length * (4 if kind == 11 else 8))
        else:
            raise NBTError(f"Unsupported NBT tag type {kind}")
        return Tag(kind, name, start, payload_start, self.p, value, children)


def parse(data: bytes) -> Tag:
    scanner = Scanner(data)
    if scanner.number(">B") != 10:
        raise NBTError("Root NBT must be a compound")
    name = scanner.string()
    root = scanner.payload(10, name, 0, 0)
    if scanner.p != len(data):
        raise NBTError("Trailing bytes in NBT stream")
    return root


def encode_string(value: str) -> bytes:
    data = value.encode("utf-8")
    if len(data) > 65535:
        raise NBTError("NBT string too long")
    return struct.pack(">H", len(data)) + data


def make_tag(kind: int, name: str, payload: bytes) -> bytes:
    return bytes((kind,)) + encode_string(name) + payload


def string_tag(name: str, value: str) -> bytes:
    return make_tag(8, name, encode_string(value))


def long_array_tag(name: str, values: list[int]) -> bytes:
    return make_tag(12, name, struct.pack(">i", len(values)) + b"".join(
        struct.pack(">Q", value & ((1 << 64) - 1)) for value in values))


def list_tag(name: str, kind: int, payloads: list[bytes]) -> bytes:
    return make_tag(9, name, bytes((kind,)) + struct.pack(">i", len(payloads)) + b"".join(payloads))


def compound_payload(fields: list[bytes]) -> bytes:
    return b"".join(fields) + b"\x00"


def replace_spans(data: bytes, edits: list[tuple[int, int, bytes]]) -> bytes:
    result = bytearray()
    at = 0
    for start, end, payload in sorted(edits, key=lambda entry: entry[0]):
        if start < at or end < start or end > len(data):
            raise NBTError("Invalid or overlapping NBT replacement")
        result += data[at:start]
        result += payload
        at = end
    result += data[at:]
    return bytes(result)


def bytes_of(data: bytes, tag: Tag) -> bytes:
    return data[tag.start:tag.end]


def long_values(data: bytes, tag: Tag) -> list[int]:
    if tag.kind != 12:
        raise NBTError("Expected NBT long array")
    count = struct.unpack_from(">i", data, tag.payload_start)[0]
    return [struct.unpack_from(">Q", data, tag.payload_start + 4 + n * 8)[0] for n in range(count)]

# zenohstruct wire format

This document describes the bytes that `zenohstruct` puts on the wire so other
tools (debuggers, bag recorders, GUIs in other languages) can decode them.

The format is intentionally simple: **payloads are plain msgpack, attachments
are a 4-byte schema hash optionally followed by a msgpack header**. No custom
envelope, no length prefix beyond what msgpack itself provides.

## 1. Two topic flavours

### 1.1 `Topic` (structured)

Used for small structured messages (commands, state updates).

| Field      | Bytes                                              |
| ---------- | -------------------------------------------------- |
| payload    | msgpack-encoded `msgspec.Struct`                   |
| attachment | 4-byte BLAKE2b schema hash                         |
| encoding   | `application/msgpack;<StructName>@<hex-hash>`     |

The Zenoh encoding field is set on the publisher at declare time and is
**informative only** — it lets schema-unaware tools (e.g. `z_sub`, GUIs)
display the originating struct name and schema hash without decoding the
attachment. Receivers should not rely on it for validation; the attachment
hash is the authoritative version marker. `<hex-hash>` is the lowercase
hex of the same 4 bytes that appear in the attachment.

### 1.2 `BulkTopic` (raw payload + typed header)

Used for large binary payloads (images, point clouds) where you don't want to
pay the cost of encoding/decoding the whole buffer through msgpack.

| Field      | Bytes                                                            |
| ---------- | ---------------------------------------------------------------- |
| payload    | arbitrary user-managed bytes (image data, etc.)                  |
| attachment | `<4-byte BLAKE2b schema hash><msgpack-encoded header struct>`    |
| encoding   | user-defined (defaults to `zenoh/bytes`)                         |

The encoding is not set by default since `BulkTopic` payloads are
user-managed. Pass `encoding=zenoh.Encoding(...)` to `BulkTopic(...)` for a
declare-time default, or `encoding=...` per call to `Session.put_bulk(...)`
to override.

## 2. msgspec.Struct encoding

By default `msgspec.msgpack.encode(struct)` serialises a `Struct` as a
**msgpack array (fixarray / array16 / array32)**, not a map. The elements are
the field values in **declaration order**.

Example:

```python
class TwistCmd(msgspec.Struct):
    linear_x: float
    linear_y: float
    angular_z: float
    stamp_ns: int
```

An instance `TwistCmd(0.5, 0.0, 0.1, 1700000000000000000)` is encoded as the
msgpack array `[0.5, 0.0, 0.1, 1700000000000000000]`. Field names do **not**
appear in the wire bytes. To decode, your tool must already know the schema
(field names, types, order) — typically by sharing the schema definition or
by looking it up from the schema hash.

If `array_like=False` and the struct was instead encoded with
`msgspec.msgpack.encode(struct)` after constructing it with `array_like=True`
turned off (the default for `Struct`), it is still the array form. zenohstruct
does not use the `dict_like` / map encoding.

Type mapping (msgspec → msgpack):

| Python / msgspec type   | msgpack representation                             |
| ----------------------- | -------------------------------------------------- |
| `int`                   | int (positive / negative fixint / int8..64)        |
| `float`                 | float64                                            |
| `bool`                  | true / false                                       |
| `str`                   | str (fixstr / str8 / str16 / str32)                |
| `bytes` / `bytearray`   | bin                                                |
| `list[T]` / `tuple`     | array                                              |
| `dict[K, V]`            | map                                                |
| `None`                  | nil                                                |
| nested `Struct`         | array (recursive)                                  |
| `Optional[T]` / union   | one of the encodings above, plus `nil` for `None`  |
| `Enum`                  | the underlying value (usually int or str)          |

Use any standard msgpack library (Rust: `rmp-serde` or `rmpv`, JS: `msgpack-lite`,
…) to do the actual decoding, then map array indices to your schema's fields.

## 3. Schema hash

The 4-byte hash lets receivers detect that the sender's schema disagrees with
their own. It is **BLAKE2b** with `digest_size=4`, over the UTF-8 bytes of a
canonical signature string derived from the schema.

### 3.1 Algorithm

```text
hash = blake2b(canonical(schema).encode("utf-8"), digest_size=4)
```

`canonical(t)` is defined recursively. Class names are **not** part of the
signature — only the wire shape:

| msgspec type         | canonical form                                                 |
| -------------------- | -------------------------------------------------------------- |
| `Struct`             | `struct[{name}:{canonical(type)}:req={required},...]`          |
| `Union`              | `union[{sorted canonical members joined by ","}]`              |
| `list[T]`            | `list[{canonical(T)}]`                                         |
| `set[T]`             | `set[{canonical(T)}]`                                          |
| `frozenset[T]`       | `frozenset[{canonical(T)}]`                                    |
| `tuple[A,B,C]`       | `tuple[{canonical(A)},{canonical(B)},{canonical(C)}]`          |
| `tuple[T, ...]`      | `vartuple[{canonical(T)}]`                                     |
| `dict[K, V]`         | `dict[{canonical(K)},{canonical(V)}]`                          |
| `Literal[a, b, c]`   | `literal[{sorted repr(value) joined by ","}]`                  |
| `Enum`               | `enum[{sorted "NAME=repr(value)" joined by ","}]`              |
| scalar leaf          | the msgspec type-info class name, lowercase, `"type"` stripped |

Scalar leaves: `int`, `float`, `bool`, `str`, `bytes`, `bytearray`, `datetime`,
`date`, `time`, `timedelta`, `uuid`, `decimal`, `any`, `none`, etc. — these map
to `inttype`, `floattype`, `booltype`, … with the `type` suffix removed.

Struct fields are listed in **declaration order**, with `req=True` for required
fields and `req=False` for fields that have a default. Union members are sorted
alphabetically by canonical form so `Optional[X]` and `Union[X, None]` hash the
same.

### 3.2 Reference example

The schema

```python
class TwistCmd(msgspec.Struct):
    linear_x: float
    linear_y: float
    angular_z: float
    stamp_ns: int
```

produces the canonical signature

```text
struct[linear_x:float:req=True,linear_y:float:req=True,angular_z:float:req=True,stamp_ns:int:req=True]
```

and hash `38 22 a4 f8` (`blake2b(... , digest_size=4)`).

Use this as a fixture when implementing the hashing in another language.

### 3.3 Rust outline

```rust
use blake2::{Blake2bVar, digest::{Update, VariableOutput}};

fn schema_hash(canonical: &str) -> [u8; 4] {
    let mut h = Blake2bVar::new(4).unwrap();
    h.update(canonical.as_bytes());
    let mut out = [0u8; 4];
    h.finalize_variable(&mut out).unwrap();
    out
}
```

Build the `canonical` string from your tool's own schema descriptors using the
table in §3.1. If you only want to **display** received messages and don't care
about validation, you can skip hashing entirely — just decode the payload and
ignore the attachment, or display the hash as `hex(attachment[..4])` for the
user to recognise.

## 4. Decoding cheatsheet

For a structured topic with a known schema:

1. Read the attachment.
2. Optional: compare `attachment[..4]` to your computed schema hash. Warn the
   user on mismatch instead of refusing — at debug time you usually want to
   see the bytes anyway.
3. Decode `payload` as a msgpack array.
4. Zip the array with your schema's field list to produce a named record.

For a bulk topic:

1. Read the attachment. The first 4 bytes are the schema hash.
2. Decode `attachment[4..]` as a msgpack array → header struct.
3. Treat `payload` as raw bytes; interpret it using fields from the header
   (e.g. `width`, `height`, `encoding` for an image).

## 5. Stability

The wire format is considered stable for `zenohstruct >= 0.1`. Changes to the
canonical schema signature would invalidate every hash in the field, so any
such change will be a major-version bump with a migration note.
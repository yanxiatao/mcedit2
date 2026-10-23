"""Explicit original-BE file loaders for pending imports; never guess format."""
from __future__ import annotations
from pathlib import Path
from .be_import_queue import PendingBEImport
from .structure_be import BETransferError, load_be_transfer
from .vanilla_structure_be import load_vanilla_be_structure
from .schem_be import load_schem_be
from .classic_be import load_classic_be

FORMATS = {
    "original_nbt": (".nbt", load_be_transfer),
    "java_nbt": (".nbt", load_vanilla_be_structure),
    "sponge": (".schem", load_schem_be),
    "alpha": (".schematic", load_classic_be),
}


def load_pending_be(path, world, *, origin: tuple[int, int, int],
                    format: str, skip_air=False,
                    only_replace_air=False) -> PendingBEImport:
    """Read a bounded expert BE file into memory; no world mutations."""
    if type(format) is not str or format not in FORMATS:
        raise BETransferError("Select a supported expert BE structure format")
    expected_suffix, loader = FORMATS[format]
    source = Path(path)
    if source.suffix.lower() != expected_suffix:
        raise BETransferError(f"Expected a {expected_suffix} structure file")
    if type(origin) is not tuple or len(origin) != 3 or any(
            type(v) is not int for v in origin):
        raise BETransferError("Queue destination must be an integer XYZ tuple")
    if type(skip_air) is not bool or type(only_replace_air) is not bool:
        raise BETransferError("Queue air-mask settings must be booleans")
    transfer = loader(source)
    if (getattr(world, "is_mcregion", True)
            or getattr(world, "version", None) != transfer.structure.data_version
            or getattr(world, "legacy", None) !=
               (transfer.structure.data_version < 1519)):
        raise BETransferError(
            "Queued BE import requires matching original Anvil DataVersion")
    return PendingBEImport(
        str(source), transfer, origin, skip_air=skip_air,
        only_replace_air=only_replace_air)

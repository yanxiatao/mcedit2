# NeoForge 1.21.1 runtime heightmap evidence exporter (experimental)

This is an **optional auxiliary NeoForge mod source overlay** for the Python 3
MCEdit2 migration. It is not required to edit worlds. The source and Python
JSON importer are committed. An optional official NeoForge MDK CI
has now **compiled the mod, launched a real disposable headless Minecraft
1.21.1 server, generated JSON from the running block registry, and
successfully imported it in Python**. The first runtime test exported
26,684 exact states and verified the resulting DataVersion 3955 and
SHA-256. It also checked 48 genuine saved Minecraft chunk files using
a READ-ONLY native-world audit without changing region/level.dat bytes.
This is not a test against the user's modpack or a production-world
save/reopen acceptance. No tested production binary is published.
The main Python CI separately tests importer/refusal contracts.

## Build with official MDK

Use a separate checkout of the official
[NeoForge 1.21.1 ModDevGradle MDK](https://github.com/NeoForgeMDKs/MDK-1.21.1-ModDevGradle)
with **Java 21** and the Gradle wrapper included by the MDK.

1. Clone the official MDK into a temporary, separate build directory.
2. Merge `gradle-properties-to-merge.txt` into the MDK's
   `gradle.properties` (keep its tested NeoForge/ModDevGradle versions).
3. Delete the MDK's sample `com.example.examplemod` Java source files,
   and its sample `src/main/templates/META-INF/neoforge.mods.toml`.
4. Copy this overlay's `src/main/java/org/mcedit2/evidence/...` to the
   corresponding path in the MDK. Copy its
   `src/main/resources/META-INF/neoforge.mods.toml` to the MDK's
   `src/main/resources/META-INF/`.
5. Run `./gradlew build` on Linux or `gradlew.bat build` on Windows in
   the MDK directory. If compilation fails, treat the exporter as
   **unverified** and inspect mappings/API differences; do not guess.
6. The output should be in the MDK's `build/libs/`. Install the tested
   resulting mod JAR into a disposable test-instance `mods` directory
   containing **Minecraft 1.21.1 NeoForge** and the same mods/datapacks
   as the world you plan to inspect. Do not use a production server until
   the binary is validated.

The helper does **not** auto-export at login/startup or write world NBT.
An operator with permission level 2 runs:

    /mcedit2export

It enumerates every registered block state, evaluates the Minecraft
`Heightmap.Types.OCEAN_FLOOR` and `MOTION_BLOCKING` predicates, and
checks whether the exact state has a nonempty fluid. The runtime must
satisfy `motion == ocean || fluid`; otherwise the command **fails
without producing a partial file**. The snapshot has the actual 1.21.1
DataVersion 3955 and includes modded block-state IDs and all their exact
property combinations.

The JSON is stored in the **game directory** (not the save directory) at:

    mcedit2-evidence/mcedit2-states-3955-<SHA256-prefix>.json

The command uses a temporary file and atomic rename when the filesystem
allows it. Repeating a command with identical data reuses the same
content-addressed filename. Imports are bounded to 200,000 exact states
and 32 MiB. If the modpack exceeds either limit, export aborts
rather than silently truncating or splitting the facts.

## Consume in MCEdit2 Next

Use **File > Preview collision heightmap using state evidence JSON…**
and select the newly exported file. Review the reported SHA-256 and
DataVersion and choose exact world Y bounds. The editor only creates
read-only MOTION_BLOCKING/OCEAN_FLOOR preview candidates; these are not
approved or saved. The experimental WORLD_SURFACE writer is separate.

### Semantic compatibility of the JSON

The JSON key `blocks_motion_in_heightmap` is a **legacy name in the
Python importer**. For this 1.21.1 exporter its value is precisely
`Heightmap.Types.OCEAN_FLOOR.isOpaque().test(state)`, **not a
Minecraft block-tag membership claim**. The `has_fluid` value comes
from `!state.getFluidState().isEmpty()`.
The exporter validates MOTION_BLOCKING against their logical OR.
The schema field name is retained to avoid breaking prior importer files,
but the actual game predicates take precedence over assumptions about tags.

### Safety / limitations

- This is a diagnostic Java helper, not part of Python's world writer.
- Export can be expensive on large modpacks; it runs only on operator
  command and refuses oversized registries. Do not run it repeatedly
  during busy sessions.
- Installed mods and loaded datapacks may change behavior. Verify the
  export was made using the same runtime environment as the target world.
- Even a valid export does not verify the game's lighting engine, heightmap
  metadata correctness, world save transaction, modded BE integrity or
  every version of Minecraft. Tested game-generated fixtures are future
  work.

## Automated runtime acceptance

The opt-in workflow
[Optional NeoForge 1.21.1 evidence exporter compile](../../.github/workflows/neoforge-exporter-compile.yml)
clones the official MDK, compiles the mod with Java 21, then runs a
disposable headless server. **Only when both**
`-Dmcedit2.evidence.ciExport=true` and environment variable
`MCEDIT2_EVIDENCE_CI=1` are set, a server-started hook performs one
export and stops the test server; ordinary installations ignore this hook.
No CI switch should be set for production installations.

After shutdown, `verify_runtime_export.py` checks the game-produced
JSON's SHA-256-based filename, source identity, DataVersion, minimum
registry size and representative air/stone/water/stair states.
`verify_runtime_world.py` separately opens the server's **genuine
Minecraft 1.21.1 save** with the native editor reader, samples existing
chunks, and verifies the region and level.dat bytes have not changed.
Both are non-mutating diagnostic entry points. See
[CI run 39475914856](https://github.com/yanxiatao/mcedit2/actions/runs/39475914856)
for the first green combined runtime+real-region result.

**This does not yet verify** actual edits, saves and game reopens on
worlds containing Block Entities from arbitrary mods, or enable any
collision-heightmap writer. Test user modpacks separately.

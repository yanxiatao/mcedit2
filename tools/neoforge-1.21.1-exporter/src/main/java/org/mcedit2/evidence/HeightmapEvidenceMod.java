package org.mcedit2.evidence;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonObject;
import net.minecraft.SharedConstants;
import net.minecraft.commands.Commands;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.chat.Component;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.levelgen.Heightmap;
import net.neoforged.fml.common.Mod;
import net.neoforged.fml.loading.FMLPaths;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.RegisterCommandsEvent;
import net.neoforged.neoforge.event.server.ServerStartedEvent;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HexFormat;
import java.util.List;
import java.util.TreeMap;

/**
 * Optional, server-command-triggered Minecraft 1.21.1 runtime-state exporter.
 *
 * This mod does not edit a world, inspect arbitrary region files, schedule
 * world mutations, provide an editor backend, or run when no command is used.
 *
 * It exports the ACTUAL runtime Heightmap.Types predicates, avoiding
 * fragile approximations based on block names or nominal voxel shapes.
 * OCEAN_FLOOR's predicate is serialized into the legacy JSON property
 * "blocks_motion_in_heightmap"; that property is NOT a Minecraft block tag.
 */
@Mod(HeightmapEvidenceMod.MOD_ID)
public final class HeightmapEvidenceMod {
    public static final String MOD_ID = "mcedit2evidence";
    private static final int EXPECTED_DATA_VERSION = 3955; // Java 1.21.1 only
    private static final int MAX_STATES = 200_000;
    private static final int MAX_OUTPUT_BYTES = 32 * 1024 * 1024;
    private static final Gson GSON = new GsonBuilder().disableHtmlEscaping().create();

    public HeightmapEvidenceMod() {
        // Commands are rebuilt after data-pack reload; this event uses the
        // effective server runtime registries and their current predicates.
        NeoForge.EVENT_BUS.addListener(HeightmapEvidenceMod::registerCommands);
        NeoForge.EVENT_BUS.addListener(HeightmapEvidenceMod::runCiSmokeOnServerStarted);
    }

    private static void registerCommands(RegisterCommandsEvent event) {
        event.getDispatcher().register(
            Commands.literal("mcedit2export")
                .requires(source -> source.hasPermission(2))
                .executes(context -> {
                    try {
                        Path output = writeSnapshot();
                        context.getSource().sendSuccess(
                            () -> Component.literal(
                                "MCEdit2: runtime state evidence exported to "
                                + output.toAbsolutePath()), false);
                        return 1;
                    } catch (Exception ex) {
                        context.getSource().sendFailure(Component.literal(
                            "MCEdit2 export refused: " + ex.getMessage()));
                        return 0;
                    }
                })
        );
    }

    /**
     * Opt-in CI runtime smoke gate. Requires BOTH a dedicated JVM property
     * and a runner-only environment variable. An ordinary installation
     * never exports at startup and continues using /mcedit2export.
     *
     * This only runs on a disposable development server in a CI job.
     */
    private static void runCiSmokeOnServerStarted(ServerStartedEvent event) {
        if (!Boolean.getBoolean("mcedit2.evidence.ciExport")
                || !"1".equals(System.getenv("MCEDIT2_EVIDENCE_CI"))) {
            return;
        }
        try {
            Path result = writeSnapshot();
            System.out.println("MCEDIT2_CI_EVIDENCE_OK " + result.toAbsolutePath());
        } catch (Exception ex) {
            System.err.println("MCEDIT2_CI_EVIDENCE_FAILED " + ex);
            // A failure is externally detectable because no valid JSON is
            // created, even when Minecraft exits with success status.
        } finally {
            // Only exit an explicitly double-gated disposable test server.
            event.getServer().halt(false);
        }
    }

    private static Path writeSnapshot() throws IOException, NoSuchAlgorithmException {
        int version = SharedConstants.getCurrentVersion()
            .getDataVersion().getVersion();
        if (version != EXPECTED_DATA_VERSION) {
            throw new IllegalStateException(
                "This exporter is only for Minecraft 1.21.1 DataVersion 3955; got "
                + version);
        }

        TreeMap<String, JsonObject> facts = new TreeMap<>();
        for (Block block : BuiltInRegistries.BLOCK) {
            String id = BuiltInRegistries.BLOCK.getKey(block).toString();
            for (BlockState state : block.getStateDefinition().getPossibleStates()) {
                String key = serializeState(id, state);
                boolean ocean = Heightmap.Types.OCEAN_FLOOR.isOpaque().test(state);
                boolean fluid = !state.getFluidState().isEmpty();
                boolean motion = Heightmap.Types.MOTION_BLOCKING.isOpaque().test(state);
                if (motion != (ocean || fluid)) {
                    throw new IllegalStateException(
                        "Heightmap predicate mismatch for " + key
                        + "; exporter/schema cannot represent this runtime");
                }
                JsonObject entry = new JsonObject();
                // Historical JSON key kept for compatibility with Python
                // parser. Semantics = actual OCEAN_FLOOR predicate result.
                entry.addProperty("blocks_motion_in_heightmap", ocean);
                entry.addProperty("has_fluid", fluid);
                if (facts.putIfAbsent(key, entry) != null) {
                    throw new IllegalStateException("Duplicate native state " + key);
                }
                if (facts.size() > MAX_STATES) {
                    throw new IllegalStateException(
                        "Runtime has more than " + MAX_STATES
                        + " states; aborting instead of exporting partial evidence");
                }
            }
        }
        if (facts.isEmpty()) {
            throw new IllegalStateException("No native block states were registered");
        }

        JsonObject out = new JsonObject();
        out.addProperty("schema", 1);
        out.addProperty("data_version", version);
        out.addProperty("source",
            "NeoForge 1.21.1 server runtime Heightmap.Types / fluid-state export");
        JsonObject states = new JsonObject();
        facts.forEach(states::add);
        out.add("states", states);
        byte[] data = GSON.toJson(out).getBytes(StandardCharsets.UTF_8);
        if (data.length > MAX_OUTPUT_BYTES) {
            throw new IllegalStateException(
                "Export exceeds 32 MiB; refusing a partial or truncated registry");
        }

        // The Minecraft game directory, NOT any save, world or region file.
        Path base = FMLPaths.GAMEDIR.get().toAbsolutePath().normalize();
        Path directory = base.resolve("mcedit2-evidence");
        if (Files.isSymbolicLink(directory)) {
            throw new IOException("Export directory must not be a symlink");
        }
        Files.createDirectories(directory);
        if (!Files.isDirectory(directory)) {
            throw new IOException("Export destination is not a directory");
        }
        String digest = HexFormat.of().formatHex(
            MessageDigest.getInstance("SHA-256").digest(data));
        Path output = directory.resolve(
            "mcedit2-states-" + version + "-" + digest.substring(0, 16) + ".json");
        if (Files.exists(output)) {
            if (!Files.isRegularFile(output) || Files.isSymbolicLink(output)
                    || Files.size(output) != data.length
                    || !MessageDigest.isEqual(
                       MessageDigest.getInstance("SHA-256")
                         .digest(Files.readAllBytes(output)),
                       MessageDigest.getInstance("SHA-256").digest(data))) {
                throw new IOException("Existing export filename has different data");
            }
            return output;
        }
        Path temp = Files.createTempFile(directory, "mcedit2-evidence-", ".tmp");
        try {
            Files.write(temp, data, StandardOpenOption.WRITE,
                StandardOpenOption.TRUNCATE_EXISTING);
            try {
                Files.move(temp, output, StandardCopyOption.ATOMIC_MOVE);
            } catch (AtomicMoveNotSupportedException e) {
                Files.move(temp, output);
            }
            return output;
        } finally {
            Files.deleteIfExists(temp);
        }
    }

    private static String serializeState(String id, BlockState state) {
        List<Property<?>> props = new ArrayList<>(state.getProperties());
        props.sort(Comparator.comparing(Property::getName));
        if (props.isEmpty()) {
            return id;
        }
        StringBuilder result = new StringBuilder(id).append('[');
        boolean first = true;
        for (Property<?> property : props) {
            if (!first) result.append(',');
            first = false;
            result.append(property.getName()).append('=');
            result.append(propertyValue(state, property));
        }
        return result.append(']').toString();
    }

    private static <T extends Comparable<T>> String propertyValue(
            BlockState state, Property<T> property) {
        return property.getName(state.getValue(property));
    }
}

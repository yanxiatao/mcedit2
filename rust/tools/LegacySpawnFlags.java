import com.google.gson.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;

/** 准确 1.12.2 客户端的直接 API 参照；不反射、不创建世界或窗口。 */
public final class LegacySpawnFlags {
    public static void main(String[] args) throws Exception {
        java.io.PrintStream stderr = System.err;
        try {
            ni.c();
        } finally {
            System.setErr(stderr);
        }
        JsonArray blocks = new JsonArray();
        for (aow block : aow.h) {
            JsonObject entry = new JsonObject();
            entry.addProperty("id", aow.a(block));
            entry.addProperty("name", aow.h.b(block).toString());
            JsonArray states = new JsonArray();
            JsonObject errors = new JsonObject();
            for (int meta = 0; meta < 16; meta++) {
                try {
                    awt state = block.a(meta);
                    // aws.l -> aow.s: material.isOpaque && isFullCube && !canProvidePower。
                    int flags = (state.l() ? 1 : 0) | (state.a().d() ? 2 : 0);
                    states.add(flags);
                } catch (IllegalArgumentException exception) {
                    // 参照输出同时记录客户端拒绝的编码，运行时据此正常报告源数据错误。
                    states.add(JsonNull.INSTANCE);
                    errors.addProperty(Integer.toString(meta), exception.toString());
                }
            }
            entry.add("states", states);
            entry.add("errors", errors);
            blocks.add(entry);
        }
        JsonObject result = new JsonObject();
        result.addProperty("data_version", 1343);
        result.addProperty("client_sha1", "0f275bc1547d01fa5f56ba34bdc87d981ee12daf");
        result.add("blocks", blocks);
        Files.write(Paths.get(args[0]), new GsonBuilder().setPrettyPrinting().create()
            .toJson(result).getBytes(StandardCharsets.UTF_8));
    }
}

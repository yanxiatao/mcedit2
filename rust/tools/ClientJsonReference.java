// 只调用 Gson 公开接口，记录单文档解析语义；不读取游戏或账户，不使用反射。
import com.google.gson.*;
import com.google.gson.stream.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
public final class ClientJsonReference {
    private static JsonObject shape(JsonElement value) {
        JsonObject result=new JsonObject();
        if(value.isJsonNull()) { result.addProperty("type","null"); return result; }
        if(value.isJsonObject()) {
            result.addProperty("type","object"); JsonObject fields=new JsonObject();
            for(Map.Entry<String,JsonElement> e:value.getAsJsonObject().entrySet()) fields.add(e.getKey(),shape(e.getValue()));
            result.add("value",fields);return result;
        }
        if(value.isJsonArray()) {
            result.addProperty("type","array");JsonArray items=new JsonArray();
            for(JsonElement item:value.getAsJsonArray()) items.add(shape(item));
            result.add("value",items);return result;
        }
        JsonPrimitive p=value.getAsJsonPrimitive();
        result.addProperty("type",p.isString()?"string":p.isBoolean()?"boolean":"number");
        result.addProperty("value",p.getAsString());return result;
    }
    public static void main(String[] args) throws IOException {
        List<String> inputs=List.of(
            "{textures:{SKIN:{url:'http://textures.minecraft.net/texture/"+"a".repeat(60)+"'}}}",
            "/* source */ { a=>TRUE; b=false; c:NULL; a: 2 } # end",
            "[1,,2;]", "[,;]", "[]", "{}", "{'text':'unicode \\uD83D\\uDE80; \\u4e2d\\u6587'}",
            "{a:'double\\\" single\\\' slash\\/ tab\\t line\\n'}", "{a:'actual\nline'}", "{a:'escaped\\\nline'}",
            "{a:'\u0001'}", "\ufeff)]}'\n {x:1}", "// only comment", "", "true", "FaLsE", "nUlL",
            "01", "+1", ".5", "1.", "-0", "1.25e+3", "1e400", "NaN", "Infinity", "-Infinity",
            "["+"9".repeat(1000)+", "+"9".repeat(1024)+"]", "{a:foo\"bar; b:'x'}",
            "{\"a\":1} trailing", "null trailing", "{a:}", "{a:1,}", "[1 2]", "{a:'\\x20'}",
            "{a:'unterminated}", "/* no end", "{a=> 1; b: [1;;;3]}", "{a = > }", "{a:1# c\n,b:2}", "{1:one; true:yes}"
        );
        JsonArray output=new JsonArray();
        for(String input:inputs) {
            JsonObject entry=new JsonObject();entry.addProperty("input",input);
            try {
                JsonReader reader=new JsonReader(new StringReader(input));reader.setLenient(true);
                JsonElement value=JsonParser.parseReader(reader);
                if(reader.peek()!=JsonToken.END_DOCUMENT) throw new JsonSyntaxException("single-document trailing data");
                entry.addProperty("accepted",true);entry.add("shape",shape(value));
            } catch(JsonParseException|IOException error) {
                entry.addProperty("accepted",false);entry.addProperty("error",error.getMessage());
            }
            output.add(entry);
        }
        Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(output),StandardCharsets.UTF_8);
    }
}

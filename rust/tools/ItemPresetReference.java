import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.file.*;
import java.util.*;
/** 准确客户端的完整创造栏物品、NBT、名称、提示及着色参照；不使用反射，不启动图形界面。 */
public final class ItemPresetReference {
 static final Map<TAG,JsonObject> values=new LinkedHashMap<>();
 static final JsonArray cases=new JsonArray();
 static COLORS_TYPE colors;
 static LOOKUP_TYPE lookup;
 BIND_METHOD
 static JsonObject record(STACK stack) throws Exception {
  if(EMPTY)throw new IllegalArgumentException("空物品参照");
  TAG tag=SAVE;
  var output=new ByteArrayOutputStream();var data=new DataOutputStream(output);WRITE;
  var preset=new JsonObject();preset.addProperty("id",ITEM_KEY);
  preset.addProperty("nbt",Base64.getEncoder().encodeToString(output.toByteArray()));
  preset.addProperty("count",COUNT);preset.addProperty("max_count",MAX_COUNT);preset.addProperty("max_damage",MAX_DAMAGE);
  preset.addProperty("foil",FOIL);
  var properties=new JsonObject();for(String key:new String[]{"damage","damaged","custom_model_data","broken","charged","firework","filled","trim_type","lefthanded","pull","pulling","cast","blocking","cooldown","level"}){var property=PROPERTY_GET;if(property!=null)properties.addProperty(key,PROPERTY_CALL);}preset.add("properties",properties);
  preset.add("name",NAME);preset.addProperty("english",ENGLISH);
  var tooltip=new JsonArray();for(var line:TOOLTIP)tooltip.add(TOOLTIP_LINE);preset.add("tooltip",tooltip);
  var tints=new JsonArray();for(int i=0;i<5;i++)tints.add(COLOR);preset.add("tints",tints);
  return preset;
 }
 static void add(STACK stack)throws Exception {if(EMPTY)return;TAG tag=SAVE;if(!values.containsKey(tag))values.put(tag,record(stack));}
 public static void main(String[]args)throws Exception {
  var stderr=System.err;try {BOOTSTRAP}finally{System.setErr(stderr);}
  colors=COLORS;LOOKUP;
  BASE_ITEMS
  CREATIVE
  COLOR_CASES
  var effects=new JsonObject();for(var effect:EFFECT_REGISTRY){var e=new JsonObject();e.addProperty("color",EFFECT_COLOR);e.addProperty("name",EFFECT_NAME);EFFECT_NUMERIC;effects.add(EFFECT_KEY,e);}
  var potions=new JsonObject();for(var potion:POTION_REGISTRY){var entries=new JsonArray();for(var effect:potion.a()){var e=new JsonObject();e.addProperty("id",POTION_EFFECT_KEY);e.addProperty("amplifier",AMPLIFIER);e.addProperty("visible",VISIBLE);entries.add(e);}potions.add(POTION_KEY,entries);}
  var result=new JsonObject();result.addProperty("version",VERSION);result.add("presets",new Gson().toJsonTree(values.values()));result.add("color_cases",cases);result.add("effects",effects);result.add("potions",potions);
  Files.writeString(Path.of(args[0]),new GsonBuilder().disableHtmlEscaping().create().toJson(result));
  System.out.println("完整物品预设："+values.size());
 }
}

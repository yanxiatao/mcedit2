"""使用准确客户端的直接 API 导出刷怪图层属性，所有错误直接报告。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import urllib.request
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True,
                        help="已缓存的官方 1.12.2 client.jar 与 version.json 所在目录")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    reference = args.reference.resolve(strict=True)
    client = reference / "client.jar"
    version = json.loads((reference / "version.json").read_text(encoding="utf-8"))
    expected = "0f275bc1547d01fa5f56ba34bdc87d981ee12daf"
    if version["id"] != "1.12.2" or hashlib.sha1(client.read_bytes()).hexdigest() != expected:
        raise ValueError("混淆 API 只适用于准确的官方 1.12.2 客户端")
    java, javac = shutil.which("java"), shutil.which("javac")
    if not java or not javac:
        raise RuntimeError("需要已有 JDK，不自动安装")
    dependencies = []
    for library in version["libraries"]:
        if library["name"].split(":")[1] not in {
            "guava", "commons-lang3", "commons-io", "gson", "fastutil", "log4j-api", "log4j-core", "netty-all"
        }:
            continue
        artifact = library["downloads"]["artifact"]
        path = reference / "libraries" / artifact["path"]
        if not path.exists():
            path.parent.mkdir(parents=True, exist_ok=True)
            data = urllib.request.urlopen(artifact["url"]).read()
            if hashlib.sha1(data).hexdigest() != artifact["sha1"]:
                raise ValueError("官方依赖摘要不符：" + artifact["path"])
            path.write_bytes(data)
        if hashlib.sha1(path.read_bytes()).hexdigest() != artifact["sha1"]:
            raise ValueError("缓存依赖摘要不符：" + artifact["path"])
        dependencies.append(str(path))
    output = args.output.resolve()
    with tempfile.TemporaryDirectory(prefix="spawn-reference-", dir=reference) as temporary:
        work = Path(temporary)
        analysis = work / "analysis.jar"
        with zipfile.ZipFile(client) as source, zipfile.ZipFile(analysis, "w") as target:
            for entry in source.infolist():
                name = entry.filename.upper()
                if name.startswith("META-INF/") and name.endswith((".SF", ".RSA", ".DSA", ".EC")):
                    continue
                target.writestr(entry, source.read(entry))
        with zipfile.ZipFile(client) as source, zipfile.ZipFile(analysis) as target:
            for name in target.namelist():
                if source.read(name) != target.read(name):
                    raise ValueError("分析副本改变了成员：" + name)
        classpath = os.pathsep.join([str(analysis)] + dependencies)
        logging = work / "logging.xml"
        logging.write_text('<Configuration><Appenders><Console name="Console" target="SYSTEM_ERR">'
                           '<PatternLayout pattern="%level: %msg%n"/></Console></Appenders>'
                           '<Loggers><Root level="warn"><AppenderRef ref="Console"/></Root></Loggers>'
                           '</Configuration>', encoding="utf-8")
        source = Path(__file__).with_name("LegacySpawnFlags.java").resolve()
        subprocess.run([javac, "-proc:none", "-encoding", "UTF-8", "-cp", classpath,
                        "-d", str(work), str(source)], check=True, cwd=work)
        output.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([java, "-Dlog4j2.formatMsgNoLookups=true",
                        "-Dlog4j.configurationFile=" + logging.as_uri(), "-cp",
                        str(work) + os.pathsep + classpath, "LegacySpawnFlags", str(output)],
                       check=True, cwd=work)
    result = json.loads(output.read_text(encoding="utf-8"))
    print("已导出", len(result["blocks"]), "种方块的 16 个 metadata 属性；临时副本和类已清理")


if __name__ == "__main__":
    main()

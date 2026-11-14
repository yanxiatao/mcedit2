"""从准确版本的本机客户端创建可校验的只读数值参照环境。"""
import hashlib
import os
import pathlib
import shutil
import subprocess
import zipfile


def run_reference(args, name, work):
    source = pathlib.Path(__file__).with_name(name + ".java").read_text(encoding="utf-8")
    return run_source_reference(args, name, source, work, "1.21.1")


def run_source_reference(args, name, source_code, work, expected_version, class_adapters=None, extra_classpath=()):
    import json

    hashes = {"1.12.2": "0f275bc1547d01fa5f56ba34bdc87d981ee12daf",
              "1.20.1": "0c3ec587af28e5a785c0b4a7b8a30f9a8f78f838",
              "1.21.1": "30c73b1c5da787909b2f73340419fdf13b9def88"}
    work.mkdir(parents=True, exist_ok=True)
    client = args.client_jar.resolve(strict=True)
    version = json.loads(args.version_json.read_text(encoding="utf-8"))
    if version["id"] != expected_version or hashlib.sha1(client.read_bytes()).hexdigest() != hashes[expected_version]:
        raise ValueError("客户端或元数据与混淆接口版本不匹配")
    output = args.output.resolve()
    if output in (client, args.version_json.resolve()) or output.suffix != ".json":
        raise ValueError("输出必须为独立 JSON 文件")
    java, javac = shutil.which("java"), shutil.which("javac")
    if not java or not javac:
        raise RuntimeError("需要已有的 JDK 21；本工具不安装运行环境")
    analysis = work / "client-analysis.jar"
    omitted = []
    adapted = {}
    adapters = class_adapters or {}
    with zipfile.ZipFile(client) as source, zipfile.ZipFile(analysis, "w", zipfile.ZIP_DEFLATED) as target:
        for entry in source.infolist():
            upper = entry.filename.upper()
            if upper.startswith("META-INF/") and upper.endswith((".SF", ".RSA", ".DSA", ".EC")):
                omitted.append(entry.filename)
                continue
            payload = source.read(entry)
            if entry.filename in adapters:
                changed = adapters[entry.filename](payload)
                adapted[entry.filename] = changed
                payload = changed
            target.writestr(entry, payload)
    if set(adapted) != set(adapters):
        raise ValueError("指定的 CPU 绘制适配成员不存在")
    with zipfile.ZipFile(client) as source, zipfile.ZipFile(analysis) as target:
        for member in target.namelist():
            if adapted.get(member, source.read(member)) != target.read(member):
                raise ValueError("分析副本改变了归档成员：" + member)
    classpath = [str(analysis)]
    for lib in version["libraries"]:
        artifact = lib.get("downloads", {}).get("artifact", {})
        path = args.libraries / artifact.get("path", "_missing")
        if path.is_file():
            classpath.append(str(path.resolve()))
    joined = os.pathsep.join(classpath)
    if extra_classpath:
        joined += os.pathsep + os.pathsep.join(str(path.resolve(strict=True)) for path in extra_classpath)
    source_file = work / (name + ".java")
    source_file.write_text(source_code, encoding="utf-8")
    subprocess.run([javac, "-proc:none", "-encoding", "UTF-8", "-cp", joined, "-d", str(work), str(source_file)], check=True, cwd=work)
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([java, "-cp", str(work) + os.pathsep + joined, name, str(output)], check=True, cwd=work)
    proof = {
        "client_sha256": hashlib.sha256(client.read_bytes()).hexdigest(),
        "omitted_signature_entries_in_analysis_copy": omitted,
        "all_other_members_identical": not adapted,
        "output_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
    }
    if adapted:
        with zipfile.ZipFile(client) as source:
            proof["cpu_draw_adapters"] = [{"member":member,"original_sha256":hashlib.sha256(source.read(member)).hexdigest(),"adapted_sha256":hashlib.sha256(payload).hexdigest()} for member,payload in adapted.items()]
        proof["all_non_adapter_members_identical"] = True
    return proof

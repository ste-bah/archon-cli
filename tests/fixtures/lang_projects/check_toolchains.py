"""Required live probes and offline builds; failures are never skips."""
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def probe(tool, args, env, executable=None):
    executable = executable or shutil.which(tool, path=env.get("PATH"))
    if not executable or not Path(executable).is_file():
        raise RuntimeError(f"toolchain missing: {tool}")
    result = subprocess.run([str(executable), *args], capture_output=True, text=True, env=env)
    output = (result.stdout + result.stderr).strip()
    if result.returncode:
        raise RuntimeError(f"toolchain missing: {tool} (unusable: {output})")
    print(f"{tool}: {output.splitlines()[0] if output else '(no version output)'}", flush=True)
    return executable


def main():
    with tempfile.TemporaryDirectory(prefix="toolchain-probe-", dir=ROOT) as tmp:
        env = os.environ.copy()
        env.update(GRADLE_USER_HOME=str(Path(tmp) / "gradle"), MAVEN_OPTS="-Xmx128m")
        home = env.get("JAVA_HOME")
        if not home:
            raise RuntimeError("toolchain missing: java (JAVA_HOME must be set in the environment)")
        suffix = ".exe" if os.name == "nt" else ""
        env["PATH"] = str(Path(home) / "bin") + os.pathsep + env.get("PATH", "")
        probe("java", ["-version"], env, Path(home) / f"bin/java{suffix}")
        probe("javac", ["-version"], env, Path(home) / f"bin/javac{suffix}")
        probe("go", ["version"], env)
        for tool, folder, args in [
            ("mvn", "java-maven", ["-o", f"-Dmaven.repo.local={tmp}/m2", "test"]),
            ("gradle", "java-gradle", ["--offline", "--no-daemon", "test"]),
        ]:
            executable = probe(tool, ["--version"], env)
            result = subprocess.run([executable, *args], cwd=ROOT / folder,
                                    capture_output=True, text=True, env=env)
            output = result.stdout + result.stderr
            # Require actual test execution, not an unrelated failed build.
            expected = "Tests run:" if tool == "mvn" else "tests completed"
            assert result.returncode != 0 and "CalculatorTest" in output and expected in output, output
            print(f"live {folder}: failing test executed (exit {result.returncode})", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, AssertionError, OSError) as error:
        raise SystemExit(str(error))

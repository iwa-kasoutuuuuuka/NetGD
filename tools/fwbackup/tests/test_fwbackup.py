import contextlib
import hashlib
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))
import fwbackup as fb  # noqa: E402

FAKE = str(HERE / "fake_probe_rs.py")


class FakeEnv(unittest.TestCase):
    """Sets up a temp state dir and a ProbeRs pointing at the fake."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.state = Path(self.tmp.name) / "state"
        self.state.mkdir()
        self.outdir = Path(self.tmp.name) / "out"
        self._saved = dict(os.environ)
        self.addCleanup(self._restore)
        os.environ["FAKE_STATE"] = str(self.state)
        os.environ["FWBACKUP_TEST_WRAPPER"] = "1"
        for k in ("FAKE_SCENARIO", "FAKE_FMT", "FAKE_FLAKY", "FAKE_ALWAYS_FLAKY", "FAKE_FLASH",
                  "FAKE_VERSION", "FAKE_CIDR", "FWBACKUP_PROBE_RS"):
            os.environ.pop(k, None)
        # isolate from any real .probe-rs.* in the developer's home / cwd
        self.home = Path(self.tmp.name) / "home"
        self.home.mkdir()
        self.cwd = Path(self.tmp.name) / "cwd"
        self.cwd.mkdir()
        os.environ["HOME"] = os.environ["USERPROFILE"] = str(self.home)
        old = os.getcwd()
        os.chdir(self.cwd)
        self.addCleanup(os.chdir, old)

    def _restore(self):
        os.environ.clear()
        os.environ.update(self._saved)

    def pr(self):
        return fb.ProbeRs([sys.executable, FAKE])

    def scen(self, name, **kw):
        os.environ["FAKE_SCENARIO"] = name
        for k, v in kw.items():
            os.environ["FAKE_" + k.upper()] = v

    def calls(self):
        p = self.state / "calls.log"
        return p.read_text().splitlines() if p.exists() else []

    def run_main(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = fb.main(argv, pr=self.pr())
        return rc, out.getvalue(), err.getvalue()


class TestAllowlist(unittest.TestCase):
    def refuse(self, *args):
        with self.assertRaises(fb.SafetyRefusal, msg=str(args)):
            fb.validate_args(list(args))

    def test_refuses_dangerous_subcommands(self):
        for sub in ("download", "erase", "write", "run", "reset", "attach", "gdb", "serve", "debug", "trace"):
            self.refuse(sub, "--chip", "ATSAM3U2E")
            self.refuse(sub)

    def test_refuses_dangerous_flags(self):
        self.refuse("read", "--connect-under-reset", "b32", "0x80000", "4")
        self.refuse("read", "--connect-under-reset=true", "b32", "0x80000", "4")
        self.refuse("info", "--connect-under-reset")
        self.refuse("read", "--allow-erase-all", "b32", "0x80000", "4")
        self.refuse("read", "--cycle-power", "b32", "0x80000", "4")
        self.refuse("read", "-o", "x.bin", "b32", "0x80000", "4")
        self.refuse("read", "--output", "x.bin", "b32", "0x80000", "4")
        self.refuse("read", "--format", "binary", "b32", "0x80000", "4")
        self.refuse("read", "--chip-description-path", "evil.yaml", "b32", "0x80000", "4")

    def test_refuses_unknown_and_empty(self):
        self.refuse()
        self.refuse("chip", "list")
        self.refuse("list", "extra")
        self.refuse("read", "--bogus", "b32", "0x80000", "4")
        self.refuse("read", "--probe", "not-a-selector", "b32", "0x80000", "4")
        self.refuse("read", "--protocol", "jtag", "b32", "0x80000", "4")
        self.refuse("read", "--chip", "STM32F4; rm", "b32", "0x80000", "4")
        self.refuse("--version", "extra")

    def test_read_address_windows(self):
        fb.validate_args(["read", "b32", "0x00080000", "32768"])  # whole flash ok
        fb.validate_args(["read", "b32", "0x400E0740", "2"])
        self.refuse("read", "b32", "0x00080000", "32769")       # one word past the end
        self.refuse("read", "b32", "0x0009FFFC", "2")
        self.refuse("read", "b32", "0x20000000", "4")            # SRAM
        self.refuse("read", "b32", "0x400E0748", "1")
        self.refuse("read", "b32", "0x400E0740", "3")
        self.refuse("read", "b32", "0x80001", "1")               # unaligned
        self.refuse("read", "b32", "0x80000")                    # missing words
        self.refuse("read", "b99", "0x80000", "1")

    def test_allowed_forms(self):
        fb.validate_args(["--version"])
        fb.validate_args(["list"])
        fb.validate_args(["info", "--verbose", "--protocol", "swd", "--non-interactive", "--probe", "2e8a:000c"])
        fb.validate_args(["read", "--protocol", "swd", "--non-interactive", "--chip", "ATSAM3U2E",
                          "--probe", "2e8a:000c:E661", "b32", "0x00080000", "1024"])

    def test_wrapper_refuses_before_spawn(self):
        marker = []
        pr = fb.ProbeRs(["/nonexistent/should-never-run"], log=marker.append)
        for bad in (["erase"], ["download", "x.elf"], ["read", "--connect-under-reset", "b32", "0x80000", "1"]):
            with self.assertRaises(fb.SafetyRefusal):
                pr.run(bad)
        self.assertEqual(marker, [])  # nothing logged => nothing spawned

    def test_value_validation_is_strict(self):
        self.refuse("read", "--probe", "2e8a:000c\n", "b32", "0x80000", "4")
        self.refuse("read", "--chip", "ATSAM3U2E\n", "b32", "0x80000", "4")
        self.refuse("info", "--speed", "²")
        self.refuse("info", "--speed", "1000000")
        self.refuse("info", "--speed", "1\n")
        fb.validate_args(["info", "--speed", "1000"])
        fb.validate_args(["info", "--probe", "2e8a:000c:ABC_1.2-3"])


class TestEnvScrub(FakeEnv):
    def test_env_scrubbed(self):
        os.environ["PROBE_RS_CONNECT_UNDER_RESET"] = "1"
        os.environ["PROBE_RS_ALLOW_ERASE_ALL"] = "1"
        d = Path(self.tmp.name) / "envd"
        d.mkdir()
        script = d / "envdump.py"
        script.write_text("import os,sys\nsys.stdout.write(','.join(k for k in os.environ if k.startswith('PROBE_RS_')))\n")
        r = fb.ProbeRs([sys.executable, str(script)]).run(["list"])
        self.assertEqual(r.out, "")

    def test_runs_in_fresh_empty_cwd(self):
        d = Path(self.tmp.name) / "cwdd"
        d.mkdir()
        script = d / "cwddump.py"
        script.write_text("import os,sys\nsys.stdout.write(os.getcwd()+'|'+str(len(os.listdir('.'))))\n")
        (self.cwd / "marker.txt").write_text("x")
        r = fb.ProbeRs([sys.executable, str(script)]).run(["list"])
        cwd, n = r.out.rsplit("|", 1)
        self.assertEqual(n, "0")
        self.assertNotEqual(os.path.normcase(os.path.realpath(cwd)), os.path.normcase(os.path.realpath(self.cwd)))


class TestConfigRefusal(FakeEnv):
    def assert_refused(self, where):
        marker = []
        pr = fb.ProbeRs([sys.executable, FAKE], log=marker.append)
        with self.assertRaises(fb.SafetyRefusal) as cm:
            pr.run(["list"])
        self.assertIn(".probe-rs.", str(cm.exception))
        self.assertIn(str(where), str(cm.exception))
        self.assertEqual(marker, [])
        self.assertEqual(self.calls(), [])  # fake probe-rs never started

    def test_next_to_executable(self):
        d = Path(self.tmp.name) / "bin"
        d.mkdir()
        (d / "probe-rs-fake.py").write_text(Path(FAKE).read_text())
        (d / ".probe-rs.toml").write_text("[presets.default]\nconnect_under_reset = true\n")
        pr = fb.ProbeRs([sys.executable, str(d / "probe-rs-fake.py")])
        with self.assertRaises(fb.SafetyRefusal):
            pr.run(["--version"])
        self.assertEqual(self.calls(), [])

    def test_in_cwd(self):
        (self.cwd / ".probe-rs.yaml").write_text("x: 1\n")
        self.assert_refused(self.cwd)

    def test_in_home(self):
        (self.home / ".probe-rs.json").write_text("{}")
        self.assert_refused(self.home)

    def test_all_extensions(self):
        for ext in ("toml", "json", "yaml", "yml"):
            f = self.home / f".probe-rs.{ext}"
            f.write_text("")
            self.assert_refused(f)
            f.unlink()

    def test_main_refuses(self):
        (self.home / ".probe-rs.toml").write_text("")
        rc, _, err = self.run_main(["check"])
        self.assertEqual(rc, 99)
        self.assertIn(".probe-rs.toml", err)
        self.assertEqual(self.calls(), [])


class TestDiscover(FakeEnv):
    def test_json_list_requires_test_wrapper(self):
        os.environ["FWBACKUP_PROBE_RS"] = json.dumps([sys.executable, FAKE])
        os.environ["FWBACKUP_TEST_WRAPPER"] = "0"
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            with self.assertRaises(SystemExit) as cm:
                fb.ProbeRs()
        self.assertEqual(cm.exception.code, 2)
        self.assertIn("JSON", err.getvalue())
        os.environ["FWBACKUP_TEST_WRAPPER"] = "1"
        self.assertEqual(fb.ProbeRs().cmd, [sys.executable, FAKE])

    def test_plain_path_ok(self):
        os.environ["FWBACKUP_TEST_WRAPPER"] = "0"
        os.environ["FWBACKUP_PROBE_RS"] = FAKE
        self.assertEqual(fb.ProbeRs().cmd, [os.path.abspath(FAKE)])


class TestVersionPin(FakeEnv):
    def test_unreviewed_versions(self):
        for v in ("0.31.0", "0.32.1", "0.33.0", "0.32.0-rc1", "1.0.0"):
            self.scen("unlocked2e", version=v)
            self.assertEqual(fb.classify(self.pr(), "ATSAM3U2E", None)["status"], fb.ST_UNREVIEWED_PROBE_RS, v)
        self.assertEqual([c for c in self.calls() if not c.startswith("--version")], [])
        self.scen("unlocked2e", version="0.32.0")
        self.assertEqual(fb.classify(self.pr(), "ATSAM3U2E", None)["status"], fb.ST_UNLOCKED_2E)

    def test_exit_code_and_dump_refused(self):
        self.scen("unlocked2e", version="0.30.0")
        rc, out, _ = self.run_main(["check"])
        self.assertEqual(rc, 2)
        self.assertIn("0.32.0", out)
        rc, _, _ = self.run_main(["dump", "--out", str(self.outdir)])
        self.assertEqual(rc, 2)
        self.assertFalse(self.outdir.exists())


class TestOldProbeRs(FakeEnv):
    def test_old_probe_rs_no_plain_info_fallback(self):
        self.scen("oldrs")
        r = fb.classify(self.pr(), "ATSAM3U2E", None)
        self.assertEqual(r["status"], fb.ST_OLD_PROBE_RS)
        self.assertEqual([c for c in self.calls() if c.startswith("info")], [])
        rc, out, _ = self.run_main(["check"])
        self.assertEqual(rc, 2)
        self.assertIn("0.32.0", out)

    def test_old_detected_at_info(self):
        self.scen("oldrs")
        orig = fb.read_block

        def read_generic(*a, **k):
            raise fb.ReadFailed(1, "generic failure")
        fb.read_block = read_generic
        try:
            r = fb.classify(self.pr(), "ATSAM3U2E", None)
        finally:
            fb.read_block = orig
        self.assertEqual(r["status"], fb.ST_OLD_PROBE_RS)
        # only the --verbose form was tried; no auto-detecting plain `info`
        self.assertEqual([c for c in self.calls() if c.startswith("info")],
                         ["info --verbose --protocol swd --non-interactive"])


class TestProbeListing(FakeEnv):
    def test_all_inaccessible(self):
        self.scen("inaccessible")
        r = fb.classify(self.pr(), "ATSAM3U2E", None)
        self.assertEqual(r["status"], fb.ST_NO_PROBE)
        self.assertIn("inaccessible", r["detail"])
        self.assertEqual([c for c in self.calls() if c.startswith("read")], [])

    def test_other_revision_refused(self):
        self.scen("unlocked2e", cidr="281A0762")
        r = fb.classify(self.pr(), "ATSAM3U2E", None)
        self.assertEqual(r["status"], fb.ST_UNLOCKED_OTHER)
        self.assertIn("リビジョン", r["known_name"])
        rc, _, _ = self.run_main(["dump", "--out", str(self.outdir)])
        self.assertEqual(rc, 10)
        self.assertFalse(self.outdir.exists())

    def test_no_target_message_mentions_security_bit(self):
        self.assertIn("セキュリティビット", fb.MSG[fb.ST_NO_TARGET])
        self.assertIn("ERASE", fb.MSG[fb.ST_NO_TARGET])


class TestParser(unittest.TestCase):
    def test_hex_table(self):
        text = "00080000: 20004000 00080201 deadbeef 00000000 11223344 55667788 99aabbcc ddeeff00\n00080020: 01020304\n"
        b = fb.parse_read_output(text, 0x80000, 36)
        self.assertEqual(b[:4], bytes.fromhex("00400020"))
        self.assertEqual(b[32:36], bytes.fromhex("04030201"))

    def test_edge_formats(self):
        line = "00080000: 04030201 08070605"
        want = bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08])
        # words "04030201" -> LE bytes 01 02 03 04
        for text in (line, line + "\r\n", line.upper(), "WARN x 12345678\n" + line + "\n\n", "04030201 08070605\n"):
            self.assertEqual(fb.parse_read_output(text, 0x80000, 8), want, text)

    def test_errors(self):
        with self.assertRaises(fb.ParseError):
            fb.parse_read_output("00080000: 04030201\n", 0x80000, 8)  # truncated
        with self.assertRaises(fb.ParseError):
            fb.parse_read_output("00080000: 04030201\n00080008: 01020304\n", 0x80000, 8)  # gap
        with self.assertRaises(fb.ParseError):
            fb.parse_read_output("", 0x80000, 4)

    def test_list_parser(self):
        p = fb.parse_list_output("The following debug probes were found:\n[0]: Debugprobe on Pico (CMSIS-DAP) -- 2e8a:000c:E66141 (CMSIS-DAP)\n")
        self.assertEqual(p[0]["vidpid"], "2e8a:000c")
        self.assertEqual(fb.parse_list_output("No debug probes were found.\n"), [])

    def test_dp_visible(self):
        self.assertTrue(fb.dp_visible("Debug Port: DPv1, Designer: ARM Ltd"))
        self.assertTrue(fb.dp_visible("Detected chip: ATSAM3U2E"))
        self.assertFalse(fb.dp_visible("Error showing ARM chip information for Debug Port X: no ACK"))


class TestChunks(unittest.TestCase):
    def test_boundaries(self):
        for chunk in (4, 4096, 65536, 0x20000, 0x30000, 12, 4100):
            plan = fb.chunk_plan(fb.FLASH_BASE, fb.FLASH_SIZE, chunk)
            self.assertEqual(plan[0][0], fb.FLASH_BASE)
            self.assertEqual(sum(n for _, n in plan), fb.FLASH_SIZE)
            for (a, n), (a2, _) in zip(plan, plan[1:]):
                self.assertEqual(a + n, a2)
            self.assertTrue(all(0 < n <= chunk and n % 4 == 0 for _, n in plan))
            self.assertEqual(plan[-1][0] + plan[-1][1], fb.FLASH_END)
        self.assertEqual(len(fb.chunk_plan(fb.FLASH_BASE, fb.FLASH_SIZE, 4096)), 32)
        self.assertEqual(len(fb.chunk_plan(fb.FLASH_BASE, fb.FLASH_SIZE, 12)), 10923)  # ragged tail
        with self.assertRaises(ValueError):
            fb.chunk_plan(0, 100, 3)


class TestCheck(FakeEnv):
    def status(self, scen, **kw):
        self.scen(scen, **kw)
        return fb.classify(self.pr(), "ATSAM3U2E", None)

    def test_unlocked_2e(self):
        r = self.status("unlocked2e")
        self.assertEqual(r["status"], fb.ST_UNLOCKED_2E)
        self.assertEqual(r["cidr"], 0x281A0761)
        self.assertEqual(r["exid"], 0)

    def test_unlocked_4e(self):
        r = self.status("unlocked4e")
        self.assertEqual(r["status"], fb.ST_UNLOCKED_OTHER)
        self.assertIn("SAM3U4E", r["known_name"])

    def test_locked(self):
        self.assertEqual(self.status("locked")["status"], fb.ST_LOCKED)

    def test_no_target(self):
        self.assertEqual(self.status("notarget")["status"], fb.ST_NO_TARGET)

    def test_no_probe(self):
        self.assertEqual(self.status("noprobe")["status"], fb.ST_NO_PROBE)

    def test_multi_probe(self):
        self.assertEqual(self.status("multiprobe")["status"], fb.ST_MULTI_PROBE)

    def test_bad_chip(self):
        self.assertEqual(self.status("badchip")["status"], fb.ST_BAD_CHIP)

    def test_no_target_responded_is_not_bad_chip(self):
        # probe-rs 0.32.0 "No target responded to the debug port" must reach `info`, not BAD_CHIP_NAME
        r = self.status("noresp")
        self.assertEqual(r["status"], fb.ST_NO_TARGET)
        self.assertTrue(any(l.split()[:1] == ["info"] for l in self.calls()))

    def test_no_probe_rs(self):
        r = fb.classify(fb.ProbeRs(cmd=[]), "ATSAM3U2E", None)
        self.assertEqual(r["status"], fb.ST_NO_PROBE_RS)

    def test_exit_codes_and_japanese(self):
        for scen, code, word in (("unlocked2e", 0, "読み出し可能"), ("locked", 11, "ERASE"), ("notarget", 12, "JTAGSEL")):
            self.scen(scen)
            rc, out, _ = self.run_main(["check"])
            self.assertEqual(rc, code, scen)
            self.assertIn(word, out)

    def test_formats(self):
        for fmt in ("crlf", "upper", "noaddr", "noise", "simple"):
            os.environ["FAKE_FMT"] = fmt
            self.assertEqual(self.status("unlocked2e")["status"], fb.ST_UNLOCKED_2E, fmt)

    def test_only_allowed_subcommands_invoked(self):
        for scen in ("unlocked2e", "locked", "notarget", "noprobe"):
            self.scen(scen)
            self.run_main(["check"])
        for line in self.calls():
            first = line.split()[0]
            self.assertIn(first, ("list", "info", "read", "--version"), line)
            self.assertNotIn("connect-under-reset", line)
            self.assertIn("--non-interactive", line) if first != "--version" and first != "list" else None


class TestDump(FakeEnv):
    def dump(self, *extra):
        return self.run_main(["dump", "--out", str(self.outdir), *extra])

    def files(self):
        return sorted(p.name for p in self.outdir.iterdir()) if self.outdir.exists() else []

    def test_clean_dump(self):
        self.scen("unlocked2e")
        rc, out, err = self.dump()
        self.assertEqual(rc, 0, err)
        names = self.files()
        self.assertEqual(len(names), 3)
        binp = next(self.outdir / n for n in names if n.endswith(".bin"))
        self.assertTrue(binp.name.startswith("gdemu60_flash_281A0761_"))
        data = binp.read_bytes()
        self.assertEqual(data, __import__("fake_probe_rs").default_flash())
        meta = json.loads(next(self.outdir / n for n in names if n.endswith(".json")).read_text())
        self.assertEqual(meta["cidr"], "0x281A0761")
        self.assertEqual(meta["read_passes"], 2)
        self.assertEqual(meta["sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(len(meta["chunk_crc32"]), 32)
        sha = next(self.outdir / n for n in names if n.endswith(".sha256")).read_text()
        self.assertTrue(sha.startswith(meta["sha256"]))
        # 2 passes x 32 chunks + CIDR probes; only read/list/--version/info otherwise
        self.assertGreaterEqual(len([c for c in self.calls() if c.startswith("read")]), 64)

    def test_chunk_validation(self):
        self.scen("unlocked2e")
        for bad in ("0", "255", "4098", "65540", "-4", "1000000"):
            self.assertEqual(self.dump("--chunk", bad)[0], 2, bad)
        self.assertEqual(self.calls(), [])  # target never touched
        self.assertEqual(self.files(), [])
        for ok in ("256", "65536"):
            self.assertEqual(self.dump("--chunk", ok)[0], 0, ok)
            for p in self.outdir.iterdir():
                p.unlink()

    def test_selected_probe_recorded(self):
        self.scen("unlocked2e")
        rc, _, err = self.dump("--probe", "2e8a:000c")
        self.assertEqual(rc, 0, err)
        meta = json.loads(next(self.outdir.glob("*.json")).read_text())
        self.assertIn("2e8a:000c", meta["probe"])
        self.assertEqual(meta["probe_selector"], "2e8a:000c")

    def test_no_part_left_after_success(self):
        self.scen("unlocked2e")
        self.assertEqual(self.dump()[0], 0)
        self.assertFalse(any(n.endswith(".part") for n in self.files()))

    def test_write_failure_leaves_no_final_file(self):
        self.scen("unlocked2e")
        real = os.fsync

        def boom(fd):
            raise OSError("simulated disk failure")
        os.fsync = boom
        try:
            rc, _, err = self.dump()
        finally:
            os.fsync = real
        self.assertEqual(rc, 3)
        self.assertIn("失敗", err)
        self.assertEqual(self.files(), [])  # no .bin, no .part, no sidecars

    def test_hash_mismatch_after_write(self):
        self.scen("unlocked2e")
        real = fb.hashlib.sha256
        calls = []

        class Bad:
            def __init__(self, b=b""):
                calls.append(1)
                self._h = real(b)
                self._bad = len(calls) > 1  # first = digest of data, second = re-hash

            def hexdigest(self):
                return "0" * 64 if self._bad else self._h.hexdigest()
        fb.hashlib.sha256 = Bad
        try:
            rc, _, err = self.dump()
        finally:
            fb.hashlib.sha256 = real
        self.assertEqual(rc, 3)
        self.assertEqual(self.files(), [])

    def test_rename_does_not_overwrite(self):
        a, b = Path(self.tmp.name) / "a", Path(self.tmp.name) / "b"
        a.write_bytes(b"new")
        b.write_bytes(b"keep")
        with self.assertRaises(OSError):
            fb._rename_noreplace(a, b)
        self.assertEqual(b.read_bytes(), b"keep")

    def test_odd_chunk_size(self):
        self.scen("unlocked2e")
        rc, _, err = self.dump("--chunk", "12000")  # does not divide 128K
        self.assertEqual(rc, 0, err)

    def test_flaky_chunk_recovered(self):
        self.scen("unlocked2e", flaky="0x85010")
        rc, out, err = self.dump()
        self.assertEqual(rc, 0, err)
        binp = next(self.outdir.glob("*.bin"))
        self.assertEqual(binp.read_bytes(), __import__("fake_probe_rs").default_flash())
        meta = json.loads(next(self.outdir.glob("*.json")).read_text())
        self.assertEqual(meta["mismatched_chunks"], [5])  # 0x85010 -> chunk 5 of 4096
        self.assertIn("5", meta["rereads"])
        self.assertIn("MISMATCH", err)

    def test_unstable_fails_loudly_and_writes_nothing(self):
        self.scen("unlocked2e", always_flaky="0x86000")
        rc, out, err = self.dump("--max-rereads", "3")
        self.assertEqual(rc, 3)
        self.assertIn("stabilise", err)
        self.assertEqual(self.files(), [])

    def test_refuses_when_locked(self):
        for scen in ("locked", "notarget", "unlocked4e"):
            self.scen(scen)
            rc, _, _ = self.dump()
            self.assertNotEqual(rc, 0, scen)
            self.assertEqual(self.files(), [], scen)

    def test_never_overwrites(self):
        self.scen("unlocked2e")
        self.outdir.mkdir()
        # pre-create the files the next run would use, by freezing the timestamp
        import fwbackup
        real = fwbackup.datetime

        class Frozen(real):
            @classmethod
            def now(cls, tz=None):
                return real(2030, 1, 2, 3, 4, 5, tzinfo=tz)

        fwbackup.datetime = Frozen
        try:
            rc, _, err = self.dump()
            self.assertEqual(rc, 0, err)
            before = {p.name: p.read_bytes() for p in self.outdir.iterdir()}
            rc, _, err = self.dump()
            self.assertEqual(rc, 4)
            self.assertIn("上書きしません", err)
            self.assertEqual(before, {p.name: p.read_bytes() for p in self.outdir.iterdir()})
        finally:
            fwbackup.datetime = real

    def test_exclusive_create(self):
        # even if the existence check were bypassed, open(..., 'xb') must not truncate
        p = Path(self.tmp.name) / "x.bin"
        p.write_bytes(b"keep")
        with self.assertRaises(FileExistsError):
            open(p, "xb")
        self.assertEqual(p.read_bytes(), b"keep")

    def test_dump_only_uses_allowed_commands(self):
        self.scen("unlocked2e")
        self.dump("--chunk", "32768")
        for line in self.calls():
            self.assertEqual(line.split()[0] in ("list", "info", "read", "--version"), True, line)


class TestInspect(unittest.TestCase):
    def good(self):
        b = bytearray(os.urandom(0))  # deterministic below
        b = bytearray((i * 37 + 11) & 0xFF for i in range(fb.FLASH_SIZE))
        b[0:4] = (0x20004000).to_bytes(4, "little")
        b[4:8] = (0x00080101).to_bytes(4, "little")
        b[0x10000:] = b"\xff" * (fb.FLASH_SIZE - 0x10000)
        b[0x400:0x40A] = b"GDEMU v6.0"
        return bytes(b)

    def test_good(self):
        r = fb.inspect_image(self.good())
        self.assertEqual(r["problems"], [])
        self.assertGreaterEqual(r["blank_pages_ff"], 0x10000 // 256 - 1)
        self.assertTrue(any("GDEMU" in s for s in r["strings"]))

    def test_all_ff_and_zero(self):
        self.assertTrue(fb.inspect_image(b"\xff" * fb.FLASH_SIZE)["problems"])
        self.assertTrue(any("0x00" in p for p in fb.inspect_image(b"\x00" * fb.FLASH_SIZE)["problems"]))

    def test_vector_checks(self):
        g = bytearray(self.good())
        g[0:4] = (0x10000000).to_bytes(4, "little")
        self.assertTrue(any("SP" in p for p in fb.inspect_image(bytes(g))["problems"]))
        g = bytearray(self.good())
        g[4:8] = (0x00080200).to_bytes(4, "little")
        self.assertTrue(any("偶数" in p for p in fb.inspect_image(bytes(g))["problems"]))
        g = bytearray(self.good())
        g[4:8] = (0x00100001).to_bytes(4, "little")
        self.assertTrue(any("範囲" in p for p in fb.inspect_image(bytes(g))["problems"]))

    def test_lzma_heuristic(self):
        g = bytearray(self.good())
        g[0x2000:0x200D] = b"\x5d" + (1 << 20).to_bytes(4, "little") + b"\xff" * 8
        self.assertIn(0x2000, fb.inspect_image(bytes(g))["lzma_candidates"])

    def test_cli(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a.bin"
            p.write_bytes(self.good())
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = fb.main(["inspect", str(p)])
                self.assertEqual(rc, 0)
                self.assertNotIn("GDEMU v6.0", out.getvalue())  # strings not listed by default
                fb.main(["inspect", str(p), "--strings"])
            self.assertIn("GDEMU v6.0", out.getvalue())


if __name__ == "__main__":
    unittest.main(verbosity=2)

"""Portable integrity tests use fabricated bytes, without financial producer claims."""
from __future__ import annotations

import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

support_spec = importlib.util.spec_from_file_location(
    "evidence_export_test_support", Path(__file__).with_name("test_review_workflow.py")
)
if support_spec is None or support_spec.loader is None:
    raise ImportError("Review workflow test support loader is unavailable.")
support = importlib.util.module_from_spec(support_spec)
support_spec.loader.exec_module(support)
manifest_fixture, workflow = support.manifest_fixture, support.workflow


class EvidenceExportTests(unittest.TestCase):
    def run_fixture(self, root: Path):
        case, _ = workflow.sources()
        receipt = {"schema_version": 1, "case": case, "period": "2024-09-30",
                   "invocations": [], "files": {}, "boundary": workflow.BOUNDARY,
                   "producer_manifests": {name: manifest_fixture(name) for name in workflow.PRODUCERS}}
        snapshot = {name: ("fabricated evidence: " + name).encode()
                    for name in workflow.export_inventory(receipt)}
        receipt["files"] = {name: workflow.digest(data) for name, data in snapshot.items()}
        root.mkdir()
        for name, data in snapshot.items():
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        (root / "receipt.json").write_bytes(workflow.json_bytes(receipt))
        (root / "receipt.sha256").write_text(workflow.digest(workflow.json_bytes(receipt)), encoding="ascii")
        return receipt, snapshot

    def export_fixture(self, root: Path, output: Path):
        receipt, snapshot = self.run_fixture(root)
        projections = {name: b"Amount\n-1.20\n" for name in workflow.EXPORT_PROJECTIONS}
        with patch.object(workflow, "verify", return_value=(receipt, snapshot)), \
                patch.object(workflow, "projection", return_value=projections), \
                patch.object(workflow, "render_html", return_value="<html>Fabricated</html>"):
            workflow.export_run(root, Path("unused"), output)
        with zipfile.ZipFile(output) as archive:
            return {item.filename: archive.read(item) for item in archive.infolist()}

    def remanifest(self, members):
        result = dict(members)
        manifest = json.loads(result.pop("export-manifest.json"))
        manifest["members"] = {name: {"size": len(data), "sha256": workflow.digest(data)}
                               for name, data in result.items()}
        result["export-manifest.json"] = workflow.json_bytes(manifest)
        return result

    def test_exact_bytes_determinism_and_producer_free_verification(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "first.zip"
            members = self.export_fixture(parent / "run", output)
            self.assertEqual(output.read_bytes(), workflow.export_bytes(members))
            self.assertIn("run/inputs/self_review.json", members)
            self.assertEqual(members["projections/sample-review-run.csv"], b"Amount\n-1.20\n")
            for name in ("verify", "sources", "pinned_producer"):
                with patch.object(workflow, name, side_effect=AssertionError("not portable")):
                    result = workflow.verify_export(output)
                    self.assertEqual(result["integrity_statement"], workflow.EXPORT_STATEMENT)
            self.assertEqual(result["period"], "2024-09-30")

    def test_rendering_uses_captured_bytes_and_removes_its_temporary_run(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            run, output = parent / "run", parent / "evidence.zip"
            receipt, snapshot = self.run_fixture(run)
            captured_paths = []

            def render(captured, bin_dir):
                self.assertNotEqual(captured, run)
                captured_paths.append(captured)
                for name, data in snapshot.items():
                    self.assertEqual((captured / name).read_bytes(), data)
                # A transient replacement of the original cannot affect either consumer.
                (run / "transient.txt").write_bytes(b"other period")
                return (captured / "inputs/self_review.json").read_text(encoding="utf-8")

            def project(captured):
                self.assertEqual(captured, captured_paths[0])
                self.assertEqual((captured / "inputs/self_review.json").read_bytes(), snapshot["inputs/self_review.json"])
                (run / "transient.txt").unlink()
                return {name: b"captured\n" for name in workflow.EXPORT_PROJECTIONS}

            with patch.object(workflow, "verify", return_value=(receipt, snapshot)), \
                    patch.object(workflow, "render_html", side_effect=render), \
                    patch.object(workflow, "projection", side_effect=project):
                workflow.export_run(run, Path("unused"), output)
            self.assertFalse(captured_paths[0].exists())
            with zipfile.ZipFile(output) as archive:
                self.assertEqual(archive.read("review.html"), snapshot["inputs/self_review.json"])

    def test_failed_writes_are_cleaned_up_and_output_creation_races_preserve_other_files(self):
        for failure in ("write", "flush", "close", "fstat", "interrupt"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                parent = Path(directory)
                run, output = parent / "run", parent / "evidence.zip"
                original_open = Path.open
                original_fstat = workflow.os.fstat

                def failed_identity(descriptor):
                    metadata = original_fstat(descriptor)
                    if output.exists() and os.path.samestat(metadata, output.stat()):
                        raise OSError("fabricated fstat failure")
                    return metadata

                class Stream:
                    def __init__(self, stream):
                        self.stream = stream

                    def __enter__(self):
                        return self

                    def fileno(self):
                        return self.stream.fileno()

                    def write(self, data):
                        if failure in ("write", "interrupt"):
                            self.stream.write(data[:10])
                            if failure == "interrupt":
                                raise KeyboardInterrupt()
                            raise OSError("fabricated write failure")
                        return self.stream.write(data)

                    def flush(self):
                        if failure == "flush":
                            raise OSError("fabricated flush failure")
                        self.stream.flush()

                    def __exit__(self, *args):
                        self.stream.close()
                        if failure == "close":
                            raise OSError("fabricated close failure")

                def opening(path, *args, **kwargs):
                    stream = original_open(path, *args, **kwargs)
                    return Stream(stream) if path == output and args == ("xb",) else stream

                with patch.object(Path, "open", opening), \
                        patch.object(workflow.os, "fstat", side_effect=failed_identity) if failure == "fstat" else patch.object(workflow.os, "fstat", wraps=original_fstat):
                    with self.assertRaises((OSError, KeyboardInterrupt)):
                        self.export_fixture(run, output)
                self.assertFalse(output.exists())
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "evidence.zip"
            original_new_output = workflow.new_output

            def raced(path):
                result = original_new_output(path)
                result.write_bytes(b"belongs to another writer")
                return result

            with patch.object(workflow, "new_output", side_effect=raced), self.assertRaises(FileExistsError):
                self.export_fixture(parent / "run", output)
            self.assertEqual(output.read_bytes(), b"belongs to another writer")

    def test_descriptor_failure_never_adopts_another_writers_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "evidence.zip"
            original_fstat = workflow.os.fstat

            def descriptor_lost(descriptor):
                metadata = original_fstat(descriptor)
                if not output.exists() or not os.path.samestat(metadata, output.stat()):
                    return metadata
                workflow.os.close(descriptor)
                output.unlink()
                output.write_bytes(b"belongs to another writer")
                raise OSError("fabricated descriptor failure")

            with patch.object(workflow.os, "fstat", side_effect=descriptor_lost), self.assertRaises(OSError):
                self.export_fixture(parent / "run", output)
            self.assertTrue(output.exists())
            self.assertEqual(output.read_bytes(), b"belongs to another writer")

    def test_unlisted_physical_files_and_directories_are_refused(self):
        for extra in ("inputs/review-note.json", "inputs/unexpected.txt", "unlisted.txt", "inputs/empty-directory", "extra-directory"):
            with self.subTest(extra=extra), tempfile.TemporaryDirectory() as directory:
                parent = Path(directory)
                run, output = parent / "run", parent / "evidence.zip"
                receipt, snapshot = self.run_fixture(run)
                path = run / extra
                if "directory" in extra:
                    path.mkdir()
                else:
                    path.write_bytes(b"additional evidence")
                with patch.object(workflow, "verify", return_value=(receipt, snapshot)), self.assertRaises(ValueError):
                    workflow.export_run(run, Path("unused"), output)
                self.assertFalse(output.exists())

    def test_unlisted_file_added_during_render_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            run, output = parent / "run", parent / "evidence.zip"
            receipt, snapshot = self.run_fixture(run)

            def render(*args):
                (run / "unlisted.txt").write_bytes(b"new evidence")
                return "captured report"

            with patch.object(workflow, "verify", return_value=(receipt, snapshot)), \
                    patch.object(workflow, "render_html", side_effect=render), \
                    patch.object(workflow, "projection", return_value={name: b"captured" for name in workflow.EXPORT_PROJECTIONS}), self.assertRaises(ValueError):
                workflow.export_run(run, Path("unused"), output)
            self.assertFalse(output.exists())

    def test_initial_inventory_is_checked_before_reading_the_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            run, output = parent / "run", parent / "evidence.zip"
            self.run_fixture(run)
            (run / "receipt.json").unlink()
            (run / "receipt.json").mkdir()
            with patch.object(Path, "read_bytes", side_effect=AssertionError("read before admission")), self.assertRaisesRegex(ValueError, "non-regular"):
                workflow.export_run(run, Path("unused"), output)
            self.assertFalse(output.exists())

    def test_export_output_cannot_mutate_the_sealed_run(self):
        for relative in ("evidence.zip", "inputs/evidence.zip", "exports/evidence.zip", "../run/evidence.zip"):
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as directory:
                run = Path(directory) / "run"
                receipt, snapshot = self.run_fixture(run)
                output = run / relative
                with patch.object(workflow, "verify", return_value=(receipt, snapshot)), \
                        patch.object(workflow, "render_html", return_value="fabricated"), \
                        patch.object(workflow, "projection", return_value={name: b"captured" for name in workflow.EXPORT_PROJECTIONS}), self.assertRaisesRegex(ValueError, "outside the sealed run"):
                    workflow.export_run(run, Path("unused"), output)
                self.assertFalse(output.exists())
                self.assertFalse((run / "exports").exists())
                workflow.check_export_tree(run, workflow.fixed_export_inventory())

    def test_input_size_limits_are_checked_before_semantic_verification(self):
        for aggregate in (False, True):
            with self.subTest(aggregate=aggregate), tempfile.TemporaryDirectory() as directory:
                parent = Path(directory)
                run, output = parent / "run", parent / "evidence.zip"
                _, snapshot = self.run_fixture(run)
                names = sorted(snapshot)[:5] if aggregate else [sorted(snapshot)[0]]
                size = workflow.MAX_EXPORT_MEMBER if aggregate else workflow.MAX_EXPORT_MEMBER + 1
                for name in names:
                    (run / name).write_bytes(b"x" * size)
                with patch.object(workflow, "verify", side_effect=AssertionError("oversized input read")), self.assertRaisesRegex(ValueError, "size limits"):
                    workflow.export_run(run, Path("unused"), output)
                self.assertFalse(output.exists())

    def test_cli_refuses_unsupported_export_options_before_reading_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            script = parent / "review_workflow.py"
            script.write_bytes(Path(workflow.__file__).read_bytes())
            output = parent / "evidence.zip"
            cases = [("export", "--review-note", ["--output", str(output)])]
            cases.extend(("verify-export", option, []) for option in ("--previous", "--review-note", "--output"))
            for command, option, extra in cases:
                with self.subTest(command=command, option=option):
                    result = subprocess.run([sys.executable, "-B", str(script), command, "--run", str(parent / "missing"), option, str(parent / "unsupported"), *extra], capture_output=True, text=True, timeout=10)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(option, result.stderr)
                    self.assertNotIn("No such file", result.stderr)
                    self.assertNotIn("cannot find", result.stderr)
                    self.assertFalse(output.exists())

    @unittest.skipUnless(hasattr(os, "mkfifo"), "POSIX FIFO creation is unavailable")
    def test_receipt_fifo_is_refused_before_waiting_for_a_writer(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            run, output = parent / "run", parent / "evidence.zip"
            self.run_fixture(run)
            (run / "receipt.json").unlink()
            os.mkfifo(run / "receipt.json")
            result = subprocess.run([sys.executable, "-B", workflow.__file__, "export", "--run", str(run), "--output", str(output)], capture_output=True, text=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("non-regular", result.stderr)
            self.assertFalse(output.exists())

    def test_standalone_cli_has_no_repository_depth_dependency(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "evidence.zip"
            self.export_fixture(parent / "run", output)
            script = parent / "review_workflow.py"
            script.write_bytes(Path(workflow.__file__).read_bytes())
            result = subprocess.run([sys.executable, "-B", str(script), "verify-export", "--run", str(output)], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["members"], len(workflow.export_inventory(json.loads((parent / "run/receipt.json").read_bytes()))) + 7)
            with patch.object(workflow, "APP", Path(workflow.APP.anchor)), \
                    patch.object(sys, "argv", ["review_workflow.py", "verify-export", "--run", str(output)]), \
                    patch("sys.stdout", io.StringIO()):
                workflow.main()

    def test_non_regular_archive_descriptor_is_refused_and_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "evidence.zip"
            self.export_fixture(parent / "run", output)
            with patch.object(workflow.os, "fstat", return_value=SimpleNamespace(st_mode=stat.S_IFIFO)), self.assertRaises(ValueError):
                workflow.verify_export(output)
            output.unlink()

    @unittest.skipUnless(hasattr(os, "mkfifo"), "POSIX FIFO creation is unavailable")
    def test_fifo_is_refused_without_waiting_for_a_writer(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            fifo = parent / "evidence.zip"
            os.mkfifo(fifo)
            result = subprocess.run([sys.executable, "-B", workflow.__file__, "verify-export", "--run", str(fifo)], capture_output=True, text=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("regular file", result.stderr)

    def test_tampering_and_nested_receipt_binding_are_distinct(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "evidence.zip"
            members = self.export_fixture(parent / "run", output)
            for rebind in (False, True):
                changed = members | {"run/inputs/self_review.json": b"changed"}
                if rebind:
                    changed = self.remanifest(changed)
                output.write_bytes(workflow.export_bytes(changed))
                with self.subTest(rebind=rebind), self.assertRaises(ValueError):
                    workflow.verify_export(output)
            # A wholly replaced internally consistent archive cannot prove origin.
            changed = members | {"review.html": b"<html>Replacement</html>"}
            output.write_bytes(workflow.export_bytes(self.remanifest(changed)))
            self.assertEqual(workflow.verify_export(output)["integrity_statement"], workflow.EXPORT_STATEMENT)

    def test_extra_missing_and_unsafe_members_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "evidence.zip"
            original = self.export_fixture(parent / "run", output)
            for name in ("extra.txt", "../escape", "run/../escape", "run\\escape", "/absolute",
                         "C:/escape", "run/./escape", "RUN/receipt.json", "run/inputs/.env"):
                output.write_bytes(workflow.export_bytes(self.remanifest(original | {name: b"extra"})))
                with self.subTest(name=name), self.assertRaises(ValueError):
                    workflow.verify_export(output)
            changed = dict(original)
            changed.pop("review.html")
            output.write_bytes(workflow.export_bytes(self.remanifest(changed)))
            with self.assertRaises(ValueError):
                workflow.verify_export(output)

    def test_zip_metadata_duplicates_links_and_size_bounds_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "evidence.zip"
            members = self.export_fixture(parent / "run", output)
            good = output.read_bytes()
            for mode in ("duplicate", "compressed", "link", "comment", "extra"):
                buffer = io.BytesIO()
                with zipfile.ZipFile(buffer, "w") as archive:
                    for name, data in members.items():
                        info = zipfile.ZipInfo(name)
                        info.create_system = 3
                        info.external_attr = ((stat.S_IFLNK if mode == "link" else stat.S_IFREG) | 0o600) << 16
                        if mode == "compressed":
                            info.compress_type = zipfile.ZIP_DEFLATED
                        if mode == "extra":
                            info.extra = b"\x01\x00\x00\x00"
                        archive.writestr(info, data)
                    if mode == "duplicate":
                        archive.writestr("review.html", b"duplicate")
                    if mode == "comment":
                        archive.comment = b"unbound comment"
                output.write_bytes(buffer.getvalue())
                with self.subTest(mode=mode), self.assertRaises(ValueError):
                    workflow.verify_export(output)
            for bad in (b"preamble" + good, good + b"trailing bytes", b"not a zip"):
                output.write_bytes(bad)
                with self.assertRaises(ValueError):
                    workflow.verify_export(output)
            output.write_bytes(good)
            for constant in ("MAX_EXPORT_FILES", "MAX_EXPORT_MEMBER", "MAX_EXPORT_BYTES"):
                with patch.object(workflow, constant, 1), self.assertRaises(ValueError):
                    workflow.verify_export(output)

    def test_manifest_schema_and_canonical_encoding_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            output = parent / "evidence.zip"
            members = self.export_fixture(parent / "run", output)
            original = json.loads(members["export-manifest.json"])
            malformed = [original | {"schema_version": True}, original | {"boundary": "approved"},
                         original | {"extra": "field"}, original | {"integrity_statement": "approved"}]
            row = next(iter(original["members"]))
            malformed.append(original | {"members": original["members"] | {row: {"size": True, "sha256": "0" * 64}}})
            for manifest in malformed:
                output.write_bytes(workflow.export_bytes(members | {"export-manifest.json": workflow.json_bytes(manifest)}))
                with self.assertRaises(ValueError):
                    workflow.verify_export(output)
            for bad in (json.dumps(original).encode(), b'{"format":1,"format":2}', b'{"items":NaN}'):
                output.write_bytes(workflow.export_bytes(members | {"export-manifest.json": bad}))
                with self.assertRaises(ValueError):
                    workflow.verify_export(output)

    def test_export_refuses_unsealed_inventory_source_changes_and_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            run, output = parent / "run", parent / "evidence.zip"
            receipt, snapshot = self.run_fixture(run)
            for extra in ("inputs/review-note.json", "inputs/unexpected.txt"):
                with patch.object(workflow, "verify", return_value=(receipt, snapshot | {extra: b"note"})), self.assertRaises(ValueError):
                    workflow.export_run(run, Path("unused"), output)
                self.assertFalse(output.exists())
            projections = {name: b"fixture" for name in workflow.EXPORT_PROJECTIONS}
            with patch.object(workflow, "verify", side_effect=[(receipt, snapshot), (receipt, snapshot | {"changed": b"x"})]), \
                    patch.object(workflow, "projection", return_value=projections), \
                    patch.object(workflow, "render_html", return_value="fixture"), self.assertRaises(ValueError):
                workflow.export_run(run, Path("unused"), output)
            self.assertFalse(output.exists())
            output.write_bytes(b"belongs to caller")
            with self.assertRaises(ValueError):
                workflow.export_run(run, Path("unused"), output)
            self.assertEqual(output.read_bytes(), b"belongs to caller")


if __name__ == "__main__":
    unittest.main()

import init, { ByteArchive, BytePackage, ByteBundle, ByteInstaller, create_file, create_encrypted_file, create_encrypted_archive_file } from './pkg/archive_wasm.js';
import * as wasm from './pkg/archive_wasm.js';
import { openBlobArchive, extractBlobEntry } from './js/range-worker.js';

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

self.onmessage = async () => {
  let stage = 'initialization';
  try {
    await init();
    const payload = new TextEncoder().encode('browser archive payload');
    const profiles = [];
    for (const format of ['zip', 'tar', 'tar.gz', 'cab', '7z', 'xz', 'tar.xz']) {
      stage = format;
      const bytes = create_file(format, 'payload.txt', payload, 1024n * 1024n);
      const archive = new ByteArchive(bytes, 1024n * 1024n, 1024n * 1024n);
      try {
        const entries = JSON.parse(archive.entries_json());
        assert(entries.length === 1 && (format === 'xz' || entries[0].name === 'payload.txt'), `${format} metadata`);
        const decoded = archive.read_entry(0, 1024n);
        assert(decoded.length === payload.length && decoded.every((byte, i) => byte === payload[i]), `${format} payload`);
        archive.test();
        let limited = false;
        try { archive.read_entry(0, 1n); } catch (_) { limited = true; }
        assert(limited, `${format} allocation limit`);
        profiles.push(format);
      } finally {
        archive.free();
      }
    }
    for (const format of ['gzip', 'zlib', 'deflate', 'lzma', 'bzip2', 'brotli', 'tar.bz2', 'tar.br']) {
      stage = format;
      const bytes = create_file(format, 'payload.txt', payload, 1024n * 1024n);
      const archive = ByteArchive.with_format(bytes, format, 1024n * 1024n);
      try {
        const decoded = archive.read_entry(0, 1024n);
        assert(decoded.length === payload.length && decoded.every((byte, i) => byte === payload[i]), `${format} stream`);
        archive.test();
        profiles.push(format);
      } finally { archive.free(); }
    }
    for (const fixture of ['bzip2_file.7z', 'zstdmt-brotli.7z']) {
      stage = fixture;
      const response = await fetch(`./archive-fixtures/${fixture}`);
      assert(response.ok, `${fixture} fixture load`);
      const bytes = new Uint8Array(await response.arrayBuffer());
      const archive = new ByteArchive(bytes, 2n * 1024n * 1024n, 16n * 1024n * 1024n);
      try {
        const entries = JSON.parse(archive.entries_json());
        archive.test();
        const entry = entries.find(entry => entry.name === (fixture === 'bzip2_file.7z' ? 'hello.txt' : 'LICENSE'));
        assert(entry, `${fixture} metadata`);
        const decoded = new TextDecoder().decode(archive.read_entry(entry.id, 1024n * 1024n));
        assert(fixture === 'bzip2_file.7z' ? decoded === 'world\n' : decoded.includes('Apache License'), `${fixture} payload`);
      } finally { archive.free(); }
    }
    for (const compression of ['copy', 'lzma', 'lzma2', 'bzip2', 'brotli']) {
      stage = `7z-${compression}`;
      const bytes = wasm.create_sevenz_file('payload.txt', payload, compression, 1024n * 1024n);
      const archive = new ByteArchive(bytes, 1024n * 1024n, 1024n * 1024n);
      try {
        const decoded = archive.read_entry(0, 1024n);
        assert(decoded.length === payload.length && decoded.every((byte, i) => byte === payload[i]), `${compression} 7z writer`);
        archive.test();
      } finally { archive.free(); }
    }
    stage = 'packages';
    const expected = await (await fetch('./pkg/fixtures/expected.json')).json();
    const packageBytes = new Uint8Array(await (await fetch('./pkg/fixtures/valid.msix')).arrayBuffer());
    const packageReader = new BytePackage(packageBytes, 1024n * 1024n);
    try {
      packageReader.validate();
      const entries = JSON.parse(packageReader.entries_json());
      const entry = entries.find(entry => entry.name === expected.appx.payload_name);
      const payload = packageReader.read_entry(entry.id, 1024n * 1024n);
      assert(JSON.stringify([...payload]) === JSON.stringify(expected.appx.payload_bytes), 'APPX native parity');
    } finally { packageReader.free(); }
    stage = 'bundle';
    const bundleBytes = new Uint8Array(await (await fetch('./pkg/fixtures/valid.msixbundle')).arrayBuffer());
    const bundle = new ByteBundle(bundleBytes, 2n * 1024n * 1024n);
    try {
      bundle.validate();
      const packages = JSON.parse(bundle.packages_json());
      assert(packages.length === 2, 'bundle declared packages');
      assert(packages.some(packageInfo => packageInfo.file_name === 'app-x64.msix' && packageInfo.architecture === 'x64'), 'bundle x64 identity');
      let rejected = false;
      try { bundle.select('missing.msix', 1024n * 1024n); } catch (_) { rejected = true; }
      assert(rejected, 'undeclared bundle selection');
      rejected = false;
      try { bundle.select('app-x64.msix', 1n); } catch (_) { rejected = true; }
      assert(rejected, 'bundle selected input limit');
      const selected = bundle.select('app-x64.msix', 1024n * 1024n);
      try {
        selected.validate();
        const entries = JSON.parse(selected.entries_json());
        const entry = entries.find(entry => entry.name === expected.appx.payload_name);
        const decoded = selected.read_entry(entry.id, 1024n * 1024n);
        assert(JSON.stringify([...decoded]) === JSON.stringify(expected.appx.payload_bytes), 'bundle native/browser payload parity');
      } finally { selected.free(); }
    } finally { bundle.free(); }
    const corruptBundleBytes = new Uint8Array(await (await fetch('./pkg/fixtures/corrupt.msixbundle')).arrayBuffer());
    const corruptBundle = new ByteBundle(corruptBundleBytes, 2n * 1024n * 1024n);
    try {
      const selected = corruptBundle.select('app-x64.msix', 1024n * 1024n);
      try {
        let rejected = false;
        try { selected.validate(); } catch (_) { rejected = true; }
        assert(rejected, 'corrupt selected bundle payload fails validation');
      } finally { selected.free(); }
    } finally { corruptBundle.free(); }
    const corruptBytes = new Uint8Array(await (await fetch('./pkg/fixtures/corrupt.msix')).arrayBuffer());
    const corrupt = new BytePackage(corruptBytes, 1024n * 1024n);
    try {
      let rejected = false;
      try { corrupt.validate(); } catch (_) { rejected = true; }
      assert(rejected, 'APPX corruption rejected');
    } finally { corrupt.free(); }
    const installerBytes = new Uint8Array(await (await fetch('./pkg/fixtures/embedded.msi')).arrayBuffer());
    const installer = new ByteInstaller(installerBytes, 1024n * 1024n, 1000);
    try {
      const files = JSON.parse(installer.files_json());
      assert(files.length === 1, 'MSI payload mapping');
      const decoded = installer.read_file(files[0].id, 1024n * 1024n);
      assert(JSON.stringify([...decoded]) === JSON.stringify(expected.msi.payload_bytes), 'MSI native parity');
    } finally { installer.free(); }
    const password = new TextEncoder().encode('browser-fixture-password');
    const encrypted = create_encrypted_file('secret.txt', payload, password, 1024n * 1024n);
    const second = create_encrypted_file('secret.txt', payload, password, 1024n * 1024n);
    assert(encrypted.some((byte, i) => byte !== second[i]), 'fresh encryption randomness');
    const encryptedReader = ByteArchive.with_password(encrypted, password, 1024n * 1024n);
    try {
      const decoded = encryptedReader.read_entry(0, 1024n);
      assert(decoded.every((byte, i) => byte === payload[i]) && decoded.length === payload.length, 'AES browser roundtrip');
      encryptedReader.test();
    } finally { encryptedReader.free(); }
    const wrong = ByteArchive.with_password(encrypted, new Uint8Array([1]), 1024n * 1024n);
    try {
      let rejected = false;
      try { wrong.test(); } catch (_) { rejected = true; }
      assert(rejected, 'AES wrong password rejected');
    } finally { wrong.free(); }
    stage = '7z-encryption';
    for (const headers of [false, true]) {
      const bytes = create_encrypted_archive_file('7z', 'secret.txt', payload, password, 1024n * 1024n, headers);
      const archive = ByteArchive.with_password(bytes, password, 1024n * 1024n);
      try {
        const decoded = archive.read_entry(0, 1024n);
        assert(decoded.length === payload.length && decoded.every((byte, i) => byte === payload[i]), '7z AES browser parity');
        archive.test();
      } finally { archive.free(); }
    }
    stage = 'incremental-zip';
    const largePayload = new Uint8Array(16 * 1024 * 1024).fill(0x5a);
    const largeZip = create_file('zip', 'large.bin', largePayload, 32n * 1024n * 1024n);
    const rangeArchive = await openBlobArchive(wasm, new Blob([largeZip]), {
      maxInput: 32n * 1024n * 1024n, maxMetadata: 1024n * 1024n, maxDecoded: 32n * 1024n * 1024n,
    });
    try {
      let count = 0;
      let chunks = 0;
      const verified = await extractBlobEntry(rangeArchive, 0, { onChunk(bytes) {
        assert(bytes.length <= 65536 && bytes.every(byte => byte === 0x5a), 'bounded ZIP step');
        count += bytes.length;
        chunks++;
      }});
      assert(verified.verified && count === largePayload.length && chunks >= 256, 'incremental final verification');
      const controller = new AbortController();
      let cancelledBytes = 0;
      let cancelled = false;
      try {
        await extractBlobEntry(rangeArchive, 0, {signal: controller.signal, onChunk(bytes) {
          cancelledBytes += bytes.length;
          if (cancelledBytes >= 3 * 65536) controller.abort();
        }});
      } catch (error) { cancelled = error.name === 'AbortError'; }
      assert(cancelled && cancelledBytes < largePayload.length, 'huge-entry cooperative cancellation');
    } finally { rangeArchive.index.free(); }
    self.postMessage({ok: true, profiles, packages: ['msix', 'msixbundle', 'msi'], encryption: ['zip-aes256', '7z-aes256-headers'], incremental: 'zip-blob-16MiB-cancellation', worker: true});
  } catch (error) {
    self.postMessage({ok: false, stage, error: String(error)});
  }
};

// Verification harness: attaches to the running Tauri app's WebView2 over the
// Chrome DevTools Protocol and inspects the live page.
// Usage: node verify-webview.mjs <screenshot-out.png|-> <read-path>

const [, , OUT_PNG, READ_PATH] = process.argv;

function fetchTarget() {
  return fetch('http://127.0.0.1:9222/json/list').then((r) => r.json());
}

async function main() {
  const targets = await fetchTarget();
  const page = targets.find((t) => t.type === 'page');
  if (!page) throw new Error('no page target found');

  const ws = new WebSocket(page.webSocketDebuggerUrl);
  const pending = new Map();
  const events = [];
  let id = 0;

  ws.addEventListener('message', (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(JSON.stringify(msg.error)));
      else resolve(msg.result);
    } else if (msg.method === 'Runtime.exceptionThrown') {
      events.push({ kind: 'exception', text: msg.params.exceptionDetails?.text });
    } else if (msg.method === 'Runtime.consoleAPICalled') {
      const type = msg.params.type;
      if (type === 'error' || type === 'warning') {
        events.push({ kind: type, text: (msg.params.args || []).map((a) => a.value ?? a.description).join(' ') });
      }
    }
  });

  function send(method, params) {
    return new Promise((resolve, reject) => {
      const mid = ++id;
      pending.set(mid, { resolve, reject });
      ws.send(JSON.stringify({ id: mid, method, params }));
    });
  }

  await new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve);
    ws.addEventListener('error', reject);
  });

  await send('Runtime.enable');
  await send('Page.enable');

  const expression = `(async () => {
    const out = {};
    out.location = location.href;
    out.documentTitle = document.title;
    out.bridgeGlobals = {
      openFile: typeof window.openFile,
      saveFile: typeof window.saveFile,
      showSaveDialog: typeof window.showSaveDialog,
      newWindow: typeof window.newWindow,
      toggleDevTools: typeof window.toggleDevTools
    };
    out.isElectronEquivalent = window.openFile !== undefined;
    out.startCircuitFileName = window.startCircuitFileName;
    out.startCircuitTextChars = window.startCircuitText ? window.startCircuitText.length : null;
    out.invokeAvailable = typeof window.__TAURI_INTERNALS__?.invoke;

    const canvases = Array.from(document.querySelectorAll('canvas'));
    out.canvasCount = canvases.length;
    if (canvases.length) {
      out.canvasSize = canvases[0].width + 'x' + canvases[0].height;
      out.canvasHasPixels = canvases.some((c) => {
        try {
          const ctx = c.getContext('2d');
          if (!ctx) return false;
          const d = ctx.getImageData(0, 0, Math.min(c.width, 200), Math.min(c.height, 200)).data;
          for (let i = 0; i < d.length; i += 4) if (d[i + 3] !== 0) return true;
          return false;
        } catch (e) { return 'err:' + e.message; }
      });
    }

    const text = document.body.innerText || '';
    out.menuFileItems = ['New Window', 'Save As', 'Open File', 'New Blank Circuit']
      .filter((s) => text.includes(s));

    if (window.__READ_PATH__) {
      try {
        const content = await window.__TAURI_INTERNALS__.invoke('read_text_file', { path: window.__READ_PATH__ });
        out.ipcReadRoundTrip = { ok: true, firstLine: content.split('\\n')[0].trim(), chars: content.length };
      } catch (e) {
        out.ipcReadRoundTrip = { ok: false, error: String(e) };
      }
      try {
        await window.__TAURI_INTERNALS__.invoke('read_text_file', { path: 'C:/definitely/not/a/real/file.txt' });
        out.errorPropagation = 'unexpectedly succeeded';
      } catch (e) {
        out.errorPropagation = 'rejected as expected';
      }
    }
    return JSON.stringify(out);
  })()`;

  // Inject the read path as a variable before evaluating.
  await send('Runtime.evaluate', {
    expression: `window.__READ_PATH__ = ${JSON.stringify(READ_PATH || null)};`,
  });

  const res = await send('Runtime.evaluate', {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });

  if (res.exceptionDetails) {
    console.log('EVAL_EXCEPTION', JSON.stringify(res.exceptionDetails, null, 2));
  } else {
    console.log('DIAGNOSTICS ' + res.result.value);
  }

  console.log('PAGE_EVENTS ' + JSON.stringify(events));

  if (OUT_PNG && OUT_PNG !== '-') {
    const shot = await send('Page.captureScreenshot', { format: 'png' });
    const fs = await import('node:fs');
    fs.writeFileSync(OUT_PNG, Buffer.from(shot.data, 'base64'));
    console.log('SCREENSHOT ' + OUT_PNG);
  }

  ws.close();
}

main().catch((e) => {
  console.error('VERIFY_FAILED', e.message);
  process.exit(1);
});

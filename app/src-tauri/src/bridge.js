// bridge.js - Tauri replacement for the Electron preload shim.
//
// The GWT client talks to the desktop shell through a small set of globals that
// the Electron build defined in app/preload.js. This file redefines exactly the
// same surface on top of Tauri, so the Java client needs no changes at all.
//
// It is installed as a Tauri initialization script, which runs before the page's
// own scripts. That ordering is required: CirSim.isElectron() probes for
// `window.openFile` during app init, so the definition has to exist by then.
//
// Calls go through __TAURI_INTERNALS__.invoke rather than the @tauri-apps/api
// package, because the GWT site is plain static JS with no bundler.

(function () {
  function invoke(cmd, args) {
    return window.__TAURI_INTERNALS__.invoke(cmd, args);
  }

  // The path used when the caller saves without a dialog (Ctrl+S), mirroring the
  // lastSavedFilePath bookkeeping in the Electron preload.
  var lastSavedFilePath = null;

  function basename(p) {
    var i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
    return i >= 0 ? p.substring(i + 1) : p;
  }

  // CirSim.isElectron() is literally `$wnd.openFile != undefined`, so defining
  // this is what switches the app into native-file mode: File > Open/Save/Save As,
  // New Window, Ctrl+S routing and native PNG/SVG export all key off it.
  window.openFile = function (callback) {
    invoke('pick_open_file').then(function (path) {
      if (!path) return null;
      return invoke('read_text_file', { path: path }).then(function (text) {
        lastSavedFilePath = path;
        callback(text, basename(path));
      });
    }).catch(function (e) {
      window.alert(e);
    });
  };

  // CommandManager.electronSaveAs() reads `file.canceled` and
  // `file.filePath.toString()`, so this must keep Electron's result shape rather
  // than returning Tauri's plain `string | null`.
  window.showSaveDialog = function (defaultPath) {
    return invoke('pick_save_file', { defaultPath: defaultPath || null }).then(function (path) {
      if (!path) return { canceled: true };
      return { canceled: false, filePath: path };
    });
  };

  // saveFile(file, data, encoding)
  //   file === null  -> overwrite the last saved path
  //   encoding 'base64' -> raw bytes (PNG export); anything else is text
  window.saveFile = function (file, data, encoding) {
    if (file) lastSavedFilePath = file.filePath.toString();
    var path = lastSavedFilePath;
    if (!path) return;
    var write = (encoding === 'base64')
      ? invoke('write_base64_file', { path: path, data: data })
      : invoke('write_text_file', { path: path, contents: data });
    write.catch(function (e) {
      window.alert(e);
    });
  };

  window.newWindow = function () {
    invoke('new_window').catch(function (e) { console.error(e); });
  };

  window.toggleDevTools = function () {
    invoke('toggle_devtools').catch(function (e) { console.error(e); });
  };
})();

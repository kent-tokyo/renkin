// RENKIN playground search worker (PR2). Keeps find_routes_v2 -- a single
// synchronous WASM call -- off the main thread, so the page stays
// responsive (scrolling, typing, the tab not showing as "unresponsive")
// while a search runs.
//
// No cooperative cancellation exists at the WASM boundary: find_routes_v2
// is one opaque synchronous call, not something that can be interrupted
// mid-computation from JS. "Cancel" and the time-budget timeout are both
// implemented on the main thread side by terminating this whole worker
// (Worker.terminate()) and spawning a fresh one -- blunt, but correct and
// simple, and it's genuinely the only thing that actually stops a running
// WASM call from here.
let mod = null;

self.onmessage = async (e) => {
  const { type, id, payload } = e.data;

  if (type === 'init') {
    try {
      mod = await import('../pkg/renkin.js');
      await mod.default();
      self.postMessage({
        type: 'ready',
        version: mod.version ? mod.version() : 'renkin',
        capabilities: mod.capabilities ? mod.capabilities() : null,
      });
    } catch (err) {
      self.postMessage({ type: 'init-error', error: String((err && err.message) || err) });
    }
    return;
  }

  if (type === 'search') {
    if (!mod) {
      self.postMessage({ type: 'search-error', id, error: 'WASM not initialized' });
      return;
    }
    try {
      const raw = mod.find_routes_v2(
        payload.target,
        payload.depth,
        payload.maxRoutes,
        payload.beamWidth,
        payload.avoidEls,
        payload.requireEls
      );
      self.postMessage({ type: 'search-result', id, raw });
    } catch (err) {
      self.postMessage({ type: 'search-error', id, error: String((err && err.message) || err) });
    }
  }
};

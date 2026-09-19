// Runs on the audio rendering thread. Forwards each 128-sample block of the
// first input channel to a MessagePort handed over from the main thread. The
// detector itself runs in a Worker, not here: the AudioWorklet global scope
// lacks fetch and TextDecoder, which the wasm-bindgen glue needs.
class CaptureProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.out = null;
    this.port.onmessage = (e) => {
      if (e.data && e.data.port) this.out = e.data.port;
    };
  }

  process(inputs) {
    const channel = inputs[0] && inputs[0][0];
    if (channel && this.out) {
      // Copy: the input buffer is reused by the audio engine.
      const block = new Float32Array(channel);
      this.out.postMessage(block, [block.buffer]);
    }
    return true;
  }
}

registerProcessor("capture-processor", CaptureProcessor);

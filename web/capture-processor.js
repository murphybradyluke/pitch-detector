// Runs on the audio rendering thread. Forwards each 128-sample block of the
// first input channel to a MessagePort handed over from the main thread,
// stamped with the audio-clock time at the end of the block so readings can
// be placed on the same clock the click track is scheduled on. The detector
// itself runs in a Worker, not here: the AudioWorklet global scope lacks
// fetch and TextDecoder, which the wasm-bindgen glue needs.
class CaptureProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.out = null;
    // Which input channel to analyse: 0 or 1 for one side of a stereo
    // interface, -1 to mix all channels. Interfaces present inputs as stereo
    // pairs, and an instrument on input 2 alone would be halved by a mix.
    this.channel = -1;
    this.port.onmessage = (e) => {
      if (e.data && e.data.port) this.out = e.data.port;
      if (e.data && typeof e.data.channel === "number") this.channel = e.data.channel;
    };
  }

  process(inputs) {
    const input = inputs[0];
    if (!input || input.length === 0 || !this.out) return true;
    let block;
    if (this.channel >= 0 && input[this.channel]) {
      // Copy: the input buffer is reused by the audio engine.
      block = new Float32Array(input[this.channel]);
    } else if (input.length === 1) {
      block = new Float32Array(input[0]);
    } else {
      block = new Float32Array(input[0].length);
      for (const ch of input) for (let i = 0; i < block.length; i++) block[i] += ch[i];
      for (let i = 0; i < block.length; i++) block[i] /= input.length;
    }
    this.out.postMessage({ t: currentTime + block.length / sampleRate, s: block }, [block.buffer]);
    return true;
  }
}

registerProcessor("capture-processor", CaptureProcessor);

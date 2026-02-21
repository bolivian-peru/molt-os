/**
 * Voice daemon client — communicates with osmoda-voice over Unix socket.
 *
 * The voice daemon listens at /run/osmoda/voice.sock and provides
 * STT (whisper.cpp) and TTS (piper-tts) capabilities.
 */

import * as http from "node:http";

interface VoiceStatus {
  listening: boolean;
  whisper_model_loaded: boolean;
  piper_model_loaded: boolean;
  whisper_model: string;
  piper_model: string;
}

interface SpeakResponse {
  audio_path: string;
  duration_ms: number;
}

interface TranscribeResponse {
  text: string;
  duration_ms: number;
}

interface RecordResponse {
  audio_path: string;
  duration_secs: number;
  text: string | null;
  transcribe_duration_ms: number | null;
}

export class VoiceClient {
  private socketPath: string;

  constructor(socketPath: string = "/run/osmoda/voice.sock") {
    this.socketPath = socketPath;
  }

  private request(method: string, path: string, body?: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const payload = body ? JSON.stringify(body) : undefined;

      const options: http.RequestOptions = {
        socketPath: this.socketPath,
        path,
        method,
        headers: {
          "Content-Type": "application/json",
          ...(payload ? { "Content-Length": Buffer.byteLength(payload) } : {}),
        },
      };

      const req = http.request(options, (res) => {
        let data = "";
        res.on("data", (chunk) => { data += chunk; });
        res.on("end", () => {
          try {
            resolve(JSON.parse(data));
          } catch {
            resolve(data);
          }
        });
      });

      req.on("error", (err: NodeJS.ErrnoException) => {
        if (err.code === "ECONNREFUSED" || err.code === "ENOENT") {
          reject(new Error(`Voice daemon not running (socket: ${this.socketPath})`));
        } else {
          reject(err);
        }
      });

      if (payload) {
        req.write(payload);
      }
      req.end();
    });
  }

  async get(path: string): Promise<unknown> {
    return this.request("GET", path);
  }

  async post(path: string, body: unknown): Promise<unknown> {
    return this.request("POST", path, body);
  }

  /** Check if the voice daemon is reachable. */
  async isAvailable(): Promise<boolean> {
    try {
      await this.get("/voice/status");
      return true;
    } catch {
      return false;
    }
  }

  async status(): Promise<VoiceStatus> {
    return this.get("/voice/status") as Promise<VoiceStatus>;
  }

  async speak(text: string): Promise<SpeakResponse> {
    return this.post("/voice/speak", { text }) as Promise<SpeakResponse>;
  }

  async transcribe(audioPath: string): Promise<TranscribeResponse> {
    return this.post("/voice/transcribe", { audio_path: audioPath }) as Promise<TranscribeResponse>;
  }

  async record(durationSecs?: number, transcribe?: boolean): Promise<RecordResponse> {
    return this.post("/voice/record", {
      duration_secs: durationSecs ?? 5,
      transcribe: transcribe ?? true,
    }) as Promise<RecordResponse>;
  }

  async setListening(enabled: boolean): Promise<unknown> {
    return this.post("/voice/listen", { enabled });
  }
}

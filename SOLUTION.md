## 🏆 Bounty: 1 SOL

**Film a real demo of two osModa servers communicating via encrypted P2P mesh. Upload to YouTube. Earn 1 SOL.**

---

## The Demo

**Title:** "My Two AI Agents Talking to Each Other Across Continents"

**Format:** 60-90 seconds. Split-screen showing two terminals (two different servers). Real camera filming the screen.

**Hook (first 3 seconds):** "I deployed two AI agents. They found each other automatically."

### What to show:

1. **Two servers:** Deploy two osModa instances (use `spawn.os.moda` or two VPS). Show they're in different locations (different IPs, ideally different regions).
2. **Mesh pairing:** On Server A, create a mesh invite (`mesh_invite_create`). Copy the invite code. On Server B, accept it (`mesh_invite_accept`).
3. **Connection:** Show `mesh_peers` on both sides — they're connected. Show the encryption details (Noise_XX + ML-KEM-768 post-quantum).
4. **Communication:** Send a message from Agent A to Agent B through the encrypted channel. Show it arriving. Show both agents responding.
5. **The punchline:** "No API gateway. No message queue. They just talk. Post-quantum encrypted."

### What makes this demoable (verified working):

- ✅ **Real Noise_XX handshake** — 3-message exchange with X25519/ChaChaPoly/BLAKE2s
- ✅ **ML-KEM-768 post-quantum** — hybrid key exchange inside encrypted tunnel
- ✅ **Invite-based pairing** — no central server, no discovery service
- ✅ **Double-layer encryption** — Noise base layer + ChaCha20-Poly1305 PQ layer
- ✅ **Persistent rooms** — SQLite-backed, messages survive restarts, gossip sync
- ✅ **44 mesh tests passing** — real TCP connections, real crypto
- ✅ **Rate limiting** — 5 attempts/IP/60s, max 32 concurrent handshakes, 10s handshake timeout

### Requirements:

- [ ] Two real servers (different IPs minimum, different regions preferred)
- [ ] Real camera filming the screen (split-screen or switching between terminals)
- [ ] Show the invite → accept → connected flow
- [ ] Show an actual message sent and received
- [ ] Upload to YouTube (public)
- [ ] Minimum 720p
- [ ] Link osModa repo and `spawn.os.moda` in description
- [ ] Post the YouTube link in this issue

### Support:

**We will help you.** If mesh connections fail, if you hit firewall issues, if the invite flow doesn't work — ping in this issue. Maintainers will debug with you live. We'll hop on a call if needed. We want this demo to be flawless.

**Tip:** Make sure port 18800 is open on both servers for TCP mesh connections. The install script sets this up automatically.

### How to claim:

1. Comment on this issue saying you're working on it
2. Film the demo
3. Upload to YouTube
4. Post the link here
5. Receive 1 SOL to your wallet

**First valid submission wins.**

---

*osModa mesh: P2P encrypted agent-to-agent communication. Noise_XX + ML-KEM-768 hybr...
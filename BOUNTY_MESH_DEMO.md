# 🎥 Bounty: "Two AI Agents Talking Across Continents" Mesh Demo

## Overview
The **Two AI Agents Talking Across Continents** bounty rewards a 60‑90 second video that demonstrates two independent `osModa` servers discovering each other, establishing an encrypted P2P mesh, and exchanging a message.

- **Reward:** 1 SOL (first valid submission)
- **Labels:** `bounty`, `video-demo`, `testing`
- **Goal:** Real‑world validation of the mesh layer (Noise_XX + ML‑KEM‑768 hybrid) and documentation of any bugs encountered.

---

## 📹 What the video must show
1. **Two distinct servers** (different IPs, preferably different regions). Each runs a fresh `osModa` instance.
2. **Mesh invite flow**:
   - On Server A run `mesh_invite_create` and copy the invite code.
   - On Server B run `mesh_invite_accept <code>`.
3. **Connection verification**:
   - Run `mesh_peers` on both sides and show the peer listed.
   - Highlight the encryption details (Noise_XX handshake + post‑quantum ML‑KEM‑768 layer).
4. **Message exchange**:
   - Send a short text from Agent A to Agent B via the mesh channel.
   - Show the message arriving and a reply.
5. **Punchline** (optional overlay text):
   - “No API gateway. No message queue. They just talk. Post‑quantum encrypted.”

### Technical requirements shown in the demo
- Port **18800** open on both servers (default mesh TCP port).
- Handshake logs indicating the three‑message Noise_XX exchange.
- Confirmation of the hybrid key exchange (look for `ML‑KEM‑768` in logs).
- Persistent room state (SQLite) surviving a quick restart (optional but impressive).

---

## 🛠️ Setup Instructions
### 1. Provision two VPS instances
- Minimum: 1 vCPU, 512 MiB RAM, any Linux distro (Ubuntu 22.04 works well).
- Ensure they are in **different geographic regions** (e.g., AWS us‑east‑1 vs eu‑central‑1).
- Open inbound TCP **18800** in the firewall/security group.

### 2. Install `osModa`
```bash
# On each server
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/install.sh | bash
# Verify installation
osmoda --version
```
The script installs the binary to `/usr/local/bin` and sets up the default data directory `~/.osmoda`.

### 3. Start the agents
```bash
# Server A
osmoda daemon start &
# Server B
osmoda daemon start &
```
Both daemons will listen on `0.0.0.0:18800`.

### 4. Create and accept a mesh invite
```bash
# Server A (create invite)
osmoda mesh_invite_create
# Copy the printed invite code, e.g. "MESH-INVITE-XYZ123"

# Server B (accept invite)
osmoda mesh_invite_accept MESH-INVITE-XYZ123
```
You should see a success message on both sides.

### 5. Verify the peer connection
```bash
# On either server
osmoda mesh_peers
```
The output must list the remote peer’s IP and a status of `connected`.

### 6. Send a test message
```bash
# Server A (send)
osmoda mesh_send "Hello from A!"
# Server B (receive)
osmoda mesh_recv
```
The message should appear on Server B. Reply the same way from B to A.

---

## 🎬 Filming Tips
- **Split‑screen**: Use a screen‑recording tool (OBS, SimpleScreenRecorder) to capture both terminals side‑by‑side, or record each terminal separately and edit them together.
- **Resolution**: Minimum 720p (1280×720). Export as MP4.
- **Hook (first 3 s)**: Overlay text “I deployed two AI agents. They found each other automatically.”
- **Audio**: Brief narration explaining each step; keep the total length under 90 seconds.
- **Lighting**: If you film a physical monitor, ensure the screen is clearly visible and avoid glare.

---

## 📤 Submission
1. Upload the video to **YouTube** (public or unlisted). Set the title to *"Two AI Agents Talking Across Continents – osModa Mesh Demo"*.
2. In the video description, include:
   - Links to the `osModa` repo and the `spawn.os.moda` tool.
   - Your wallet address for the SOL payout.
3. Post the YouTube link as a comment on this issue.
4. The first valid submission receives the bounty.

---

## 🐞 Testing & Bug Reporting
While filming, you may encounter issues such as:
- Firewall/NAT blocking port 18800.
- Invite code not being accepted (check time sync on both servers).
- Handshake failures (inspect logs with `osmoda logs`).

If any of these occur:
1. Open a new issue with the label `[BUG]` and reference this bounty (e.g., "Found while working on #<issue‑number>").
2. Include OS, distro version, exact command output, and any relevant logs.
3. Maintainers will prioritize fixing the problem; you can pull the fix and continue filming.

---

## 🙏 Thanks!
Your effort not only earns you SOL but also strengthens `osModa` by surfacing real‑world edge cases. Happy filming!

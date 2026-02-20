{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.agentos;
in {
  options.services.agentos = {
    enable = mkEnableOption "AgentOS - AI-native operating system";

    # --- Gateway (OpenClaw) ---
    openclaw = {
      enable = mkOption { type = types.bool; default = true; description = "Enable OpenClaw Gateway"; };
      package = mkOption { type = types.package; default = pkgs.openclaw; description = "OpenClaw package"; };
      port = mkOption { type = types.port; default = 18789; description = "Gateway WebSocket port"; };
      model = mkOption { type = types.str; default = "anthropic/claude-opus-4-6"; description = "Default LLM model"; };
      configFile = mkOption { type = types.nullOr types.path; default = null; description = "OpenClaw config file"; };
    };

    # --- Agent Kernel Daemon ---
    agentd = {
      package = mkOption { type = types.package; default = pkgs.agentos-agentd; description = "agentd package"; };
      socketPath = mkOption { type = types.str; default = "/run/agentos/agentd.sock"; description = "agentd Unix socket path"; };
    };

    # --- Capability Runtime ---
    sandbox = {
      enable = mkOption { type = types.bool; default = true; description = "Enable sandbox runtime"; };
      egressProxy = {
        enable = mkOption { type = types.bool; default = true; description = "Enable egress proxy"; };
        port = mkOption { type = types.port; default = 19999; description = "Egress proxy port"; };
        defaultAllow = mkOption {
          type = types.listOf types.str;
          default = [
            "cache.nixos.org"
            "channels.nixos.org"
            "github.com"
            "api.anthropic.com"
          ];
          description = "Domains always allowed through egress proxy";
        };
      };
    };

    # --- Memory System ---
    memory = {
      enable = mkOption { type = types.bool; default = true; description = "Enable ZVEC memory system"; };
      stateDir = mkOption { type = types.path; default = "/var/lib/agentos/memory"; description = "Memory data directory"; };
      embedding = {
        model = mkOption { type = types.str; default = "nomic-embed-text-v2-moe"; description = "Embedding model name"; };
        quantization = mkOption { type = types.str; default = "Q8_0"; description = "Model quantization level"; };
        device = mkOption {
          type = types.enum [ "auto" "cpu" "cuda" "rocm" ];
          default = "auto";
          description = "Embedding compute device";
        };
      };
    };

    # --- Voice ---
    voice = {
      enable = mkOption { type = types.bool; default = false; description = "Enable voice pipeline (STT + TTS)"; };
      socketPath = mkOption { type = types.str; default = "/run/agentos/voice.sock"; description = "Voice daemon socket path"; };
      whisperModel = mkOption {
        type = types.str;
        default = "ggml-base.en.bin";
        description = "Whisper model filename (downloaded to /var/lib/agentos/voice/models/)";
      };
      piperModel = mkOption {
        type = types.str;
        default = "en_US-lessac-medium.onnx";
        description = "Piper TTS model filename";
      };
    };

    # --- Boot Splash ---
    plymouth = {
      enable = mkOption { type = types.bool; default = false; description = "Enable AgentOS Plymouth boot splash"; };
    };

    # --- State ---
    stateDir = mkOption { type = types.path; default = "/var/lib/agentos"; description = "AgentOS state directory"; };

    # --- Policy ---
    approvalRequired = mkOption {
      type = types.listOf types.str;
      default = [
        "nix.rebuild"
        "system.user.create"
        "system.user.delete"
        "system.firewall.modify"
        "system.disk.format"
        "system.reboot"
        "system.shutdown"
      ];
      description = "Operations requiring explicit user approval";
    };
  };

  config = mkIf cfg.enable {

    # ===== PLYMOUTH BOOT SPLASH =====
    boot.plymouth = mkIf cfg.plymouth.enable {
      enable = true;
      theme = "agentos";
      themePackages = [
        (pkgs.callPackage ./plymouth-theme { })
      ];
    };
    boot.consoleLogLevel = mkIf cfg.plymouth.enable 0;
    boot.kernelParams = mkIf cfg.plymouth.enable [
      "quiet" "splash" "rd.systemd.show_status=false" "rd.udev.log_level=3" "udev.log_priority=3"
    ];
    boot.initrd.verbose = mkIf cfg.plymouth.enable false;

    # ===== SYSTEM PACKAGES =====
    environment.systemPackages = with pkgs; [
      cfg.agentd.package
      pkgs.agentos-agentctl
      # Core system tools OpenClaw needs for system queries
      nodejs_22
      git
      jq
      sqlite
      ripgrep
      fd
      htop
      btop
      lsof
      strace
      curl
      wget
      file
      tree
      rsync
      zip
      unzip
      pciutils
      usbutils
      iproute2
      # Sandbox tools
      bubblewrap
    ] ++ optionals cfg.openclaw.enable [
      cfg.openclaw.package
    ] ++ optionals cfg.voice.enable [
      pkgs.openai-whisper-cpp
      pkgs.piper-tts
      pkgs.pipewire
    ];

    # ===== AGENTD SERVICE =====
    systemd.services.agentos-agentd = {
      description = "AgentOS Kernel Daemon";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "notify";
        ExecStart = "${cfg.agentd.package}/bin/agentd --socket ${cfg.agentd.socketPath} --state-dir ${cfg.stateDir}";
        Restart = "always";
        RestartSec = 3;
        WatchdogSec = 30;

        # agentd runs as root — it IS the system's brain
        RuntimeDirectory = "agentos";
        StateDirectory = "agentos";

        # Hardening (still root, but limit unnecessary capabilities)
        ProtectClock = true;
        ProtectHostname = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
      };
    };

    # ===== OPENCLAW GATEWAY =====
    systemd.services.agentos-gateway = mkIf cfg.openclaw.enable {
      description = "AgentOS Gateway (OpenClaw)";
      wantedBy = [ "multi-user.target" ];
      after = [ "agentos-agentd.service" ];
      requires = [ "agentos-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = concatStringsSep " " ([
          "${cfg.openclaw.package}/bin/openclaw"
          "gateway"
          "--port ${toString cfg.openclaw.port}"
          "--verbose"
        ] ++ optionals (cfg.openclaw.configFile != null) [
          "--config ${cfg.openclaw.configFile}"
        ]);
        Restart = "always";
        RestartSec = 5;
        StateDirectory = "agentos";
      };

      environment = {
        OPENCLAW_NIX_MODE = "1";
        NODE_ENV = "production";
        AGENTOS_SOCKET = cfg.agentd.socketPath;
        HOME = cfg.stateDir;
      };
    };

    # ===== EGRESS PROXY =====
    systemd.services.agentos-egress = mkIf cfg.sandbox.egressProxy.enable {
      description = "AgentOS Egress Proxy (domain-filtered)";
      wantedBy = [ "multi-user.target" ];
      after = [ "agentos-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${pkgs.agentos-egress}/bin/agentos-egress --port ${toString cfg.sandbox.egressProxy.port} --default-allow ${concatStringsSep "," cfg.sandbox.egressProxy.defaultAllow}";
        Restart = "always";
        RestartSec = 3;
        DynamicUser = true;
        CapabilityBoundingSet = "CAP_NET_BIND_SERVICE";
      };
    };

    # ===== VOICE DAEMON =====
    systemd.services.agentos-voice = mkIf cfg.voice.enable {
      description = "AgentOS Voice Daemon (STT + TTS)";
      wantedBy = [ "multi-user.target" ];
      after = [ "agentos-agentd.service" "pipewire.service" ];
      requires = [ "agentos-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = concatStringsSep " " [
          "${pkgs.agentos-voice}/bin/agentos-voice"
          "--socket ${cfg.voice.socketPath}"
          "--data-dir ${cfg.stateDir}/voice"
          "--whisper-bin ${pkgs.openai-whisper-cpp}/bin/whisper-cpp"
          "--whisper-model ${cfg.stateDir}/voice/models/${cfg.voice.whisperModel}"
          "--piper-bin ${pkgs.piper-tts}/bin/piper"
          "--piper-model ${cfg.stateDir}/voice/models/${cfg.voice.piperModel}"
          "--agentd-socket ${cfg.agentd.socketPath}"
        ];
        Restart = "always";
        RestartSec = 5;
        RuntimeDirectory = "agentos";
        StateDirectory = "agentos";
      };

      environment = {
        AGENTOS_SOCKET = cfg.agentd.socketPath;
      };
    };

    # ===== FIREWALL =====
    networking.firewall = {
      enable = true;
      allowedTCPPorts = [ ];  # Nothing exposed by default — gateway is localhost only
    };

    # ===== WORKSPACE SETUP =====
    system.activationScripts.agentos-workspace = ''
      mkdir -p ${cfg.stateDir}/{workspace,ledger,artifacts,memory,voice/models,voice/cache}
      mkdir -p ${cfg.stateDir}/workspace/skills

      # Symlink system skills from Nix store if package exists
      if [ -d "${pkgs.agentos-system-skills or ""}/skills" ]; then
        for skill in ${pkgs.agentos-system-skills or ""}/skills/*; do
          ln -sf "$skill" ${cfg.stateDir}/workspace/skills/$(basename "$skill") 2>/dev/null || true
        done
      fi

      # Install default templates if not present
      for tmpl in AGENTS.md SOUL.md TOOLS.md; do
        src="${./../../templates}/$tmpl"
        dst="${cfg.stateDir}/workspace/$tmpl"
        if [ -f "$src" ] && [ ! -f "$dst" ]; then
          cp "$src" "$dst"
        fi
      done
    '';
  };
}

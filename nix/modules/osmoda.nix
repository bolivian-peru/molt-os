{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.osmoda;
  # When osModa UI fronts the gateway, use the internal port
  gatewayPort = if cfg.ui.enable then cfg.openclaw.internalPort else cfg.openclaw.port;

  # Generate OpenClaw config JSON from NixOS options
  channelConfig = {
    gateway = {
      auth.mode = "none";
    };
    plugins.allow = [ "osmoda-bridge" ];
  } // optionalAttrs cfg.channels.telegram.enable {
    channels.telegram = {
      enabled = true;
    } // optionalAttrs (cfg.channels.telegram.botTokenFile != null) {
      tokenFile = toString cfg.channels.telegram.botTokenFile;
    } // optionalAttrs (cfg.channels.telegram.allowedUsers != []) {
      allowedUsers = cfg.channels.telegram.allowedUsers;
    };
  } // optionalAttrs cfg.channels.whatsapp.enable {
    channels.whatsapp = {
      enabled = true;
      credentialDir = toString cfg.channels.whatsapp.credentialDir;
    } // optionalAttrs (cfg.channels.whatsapp.allowedNumbers != []) {
      allowedNumbers = cfg.channels.whatsapp.allowedNumbers;
    };
  };

  generatedConfigFile = pkgs.writeText "openclaw-config.json" (builtins.toJSON channelConfig);

  # Use user-provided config file if set, otherwise generate from NixOS options
  effectiveConfigFile =
    if cfg.openclaw.configFile != null then cfg.openclaw.configFile
    else generatedConfigFile;
in {
  options.services.osmoda = {
    enable = mkEnableOption "osModa - AI-native operating system";

    # --- Gateway (OpenClaw) ---
    openclaw = {
      enable = mkOption { type = types.bool; default = true; description = "Enable OpenClaw Gateway"; };
      package = mkOption { type = types.package; default = pkgs.openclaw; description = "OpenClaw package"; };
      port = mkOption { type = types.port; default = 18789; description = "Gateway WebSocket port (user-facing)"; };
      internalPort = mkOption { type = types.port; default = 18790; description = "Internal gateway port (used when osModa UI fronts it)"; };
      model = mkOption { type = types.str; default = "anthropic/claude-opus-4-6"; description = "Default LLM model"; };
      configFile = mkOption { type = types.nullOr types.path; default = null; description = "OpenClaw config file"; };
    };

    # --- Agent Kernel Daemon ---
    agentd = {
      package = mkOption { type = types.package; default = pkgs.osmoda-agentd; description = "agentd package"; };
      socketPath = mkOption { type = types.str; default = "/run/osmoda/agentd.sock"; description = "agentd Unix socket path"; };
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
      stateDir = mkOption { type = types.path; default = "/var/lib/osmoda/memory"; description = "Memory data directory"; };
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
      socketPath = mkOption { type = types.str; default = "/run/osmoda/voice.sock"; description = "Voice daemon socket path"; };
      whisperModel = mkOption {
        type = types.str;
        default = "ggml-base.en.bin";
        description = "Whisper model filename (downloaded to /var/lib/osmoda/voice/models/)";
      };
      piperModel = mkOption {
        type = types.str;
        default = "en_US-lessac-medium.onnx";
        description = "Piper TTS model filename";
      };
    };

    # --- Key Daemon (Crypto Wallets) ---
    keyd = {
      enable = mkEnableOption "osModa key daemon (crypto wallets)";
      socketPath = mkOption { type = types.str; default = "/run/osmoda/keyd.sock"; description = "keyd Unix socket path"; };
      policyFile = mkOption { type = types.str; default = "${cfg.stateDir}/keyd/policy.json"; description = "Policy rules JSON file"; };
    };

    # --- Watch Daemon (SafeSwitch + Autopilot) ---
    watch = {
      enable = mkEnableOption "osModa watch daemon (SafeSwitch + autopilot)";
      socketPath = mkOption { type = types.str; default = "/run/osmoda/watch.sock"; description = "watch Unix socket path"; };
      checkInterval = mkOption { type = types.int; default = 30; description = "Watcher check interval in seconds"; };
    };

    # --- Routines Engine ---
    routines = {
      enable = mkEnableOption "osModa routines engine (background automation)";
      socketPath = mkOption { type = types.str; default = "/run/osmoda/routines.sock"; description = "routines Unix socket path"; };
      routinesDir = mkOption { type = types.str; default = "${cfg.stateDir}/routines"; description = "Directory for persisted routine definitions"; };
    };

    # --- Agent Identity (EIP-8004) ---
    agentCard = {
      enable = mkEnableOption "EIP-8004 Agent Card";
      name = mkOption { type = types.str; default = "osModa"; description = "Agent name for the card"; };
      description = mkOption { type = types.str; default = "AI-native OS agent"; description = "Agent description"; };
    };

    # --- Custom UI ---
    ui = {
      enable = mkOption { type = types.bool; default = true; description = "Enable osModa custom chat UI (fronts the OpenClaw gateway)"; };
    };

    # --- Messaging Channels ---
    channels = {
      telegram = {
        enable = mkEnableOption "Telegram messaging channel";
        botTokenFile = mkOption {
          type = types.nullOr types.path;
          default = null;
          description = "Path to file containing the Telegram bot token (e.g. /var/lib/osmoda/secrets/telegram-bot-token). Create a bot via @BotFather.";
        };
        allowedUsers = mkOption {
          type = types.listOf types.str;
          default = [];
          description = "Telegram usernames allowed to interact with the bot. Empty = no restriction.";
        };
      };

      whatsapp = {
        enable = mkEnableOption "WhatsApp messaging channel";
        credentialDir = mkOption {
          type = types.path;
          default = "/var/lib/osmoda/whatsapp";
          description = "Directory for WhatsApp session credentials (Baileys auth state)";
        };
        allowedNumbers = mkOption {
          type = types.listOf types.str;
          default = [];
          description = "Phone numbers allowed to interact (E.164 format, e.g. +1234567890). Empty = no restriction.";
        };
      };
    };

    # --- Boot Splash ---
    plymouth = {
      enable = mkOption { type = types.bool; default = false; description = "Enable osModa Plymouth boot splash"; };
    };

    # --- State ---
    stateDir = mkOption { type = types.path; default = "/var/lib/osmoda"; description = "osModa state directory"; };

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
        "wallet.send"
        "wallet.create"
        "switch.begin"
      ];
      description = "Operations requiring explicit user approval";
    };
  };

  config = mkIf cfg.enable {

    # ===== PLYMOUTH BOOT SPLASH =====
    boot.plymouth = mkIf cfg.plymouth.enable {
      enable = true;
      theme = "osmoda";
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
      pkgs.osmoda-agentctl
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
    systemd.services.osmoda-agentd = {
      description = "osModa Kernel Daemon";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${cfg.agentd.package}/bin/agentd --socket ${cfg.agentd.socketPath} --state-dir ${cfg.stateDir}";
        Restart = "always";
        RestartSec = 3;

        # agentd runs as root — it IS the system's brain
        RuntimeDirectory = "osmoda";
        StateDirectory = "osmoda";

        # Hardening (still root, but limit unnecessary capabilities)
        ProtectClock = true;
        ProtectHostname = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        RestrictNamespaces = true;
      };
    };

    # ===== OPENCLAW GATEWAY =====
    systemd.services.osmoda-gateway = mkIf cfg.openclaw.enable {
      description = "osModa Gateway (OpenClaw)";
      wantedBy = [ "multi-user.target" ];
      after = [ "osmoda-agentd.service" ];
      requires = [ "osmoda-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = concatStringsSep " " [
          "${cfg.openclaw.package}/bin/openclaw"
          "gateway"
          "--port ${toString gatewayPort}"
          "--verbose"
          "--config ${effectiveConfigFile}"
        ];
        Restart = "always";
        RestartSec = 5;
        StateDirectory = "osmoda";
      };

      environment = {
        NODE_ENV = "production";
        OSMODA_SOCKET = cfg.agentd.socketPath;
        HOME = cfg.stateDir;
      };
    };

    # ===== EGRESS PROXY =====
    systemd.services.osmoda-egress = mkIf cfg.sandbox.egressProxy.enable {
      description = "osModa Egress Proxy (domain-filtered)";
      wantedBy = [ "multi-user.target" ];
      after = [ "osmoda-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${pkgs.osmoda-egress}/bin/osmoda-egress --port ${toString cfg.sandbox.egressProxy.port} --default-allow ${concatStringsSep "," cfg.sandbox.egressProxy.defaultAllow}";
        Restart = "always";
        RestartSec = 3;
        DynamicUser = true;
        CapabilityBoundingSet = "CAP_NET_BIND_SERVICE";
        AmbientCapabilities = [ "CAP_NET_BIND_SERVICE" ];
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectClock = true;
        LockPersonality = true;
      };
    };

    # ===== KEY DAEMON (Crypto Wallets) =====
    systemd.services.osmoda-keyd = mkIf cfg.keyd.enable {
      description = "osModa Key Daemon (crypto wallets)";
      wantedBy = [ "multi-user.target" ];
      after = [ "osmoda-agentd.service" ];
      requires = [ "osmoda-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = concatStringsSep " " [
          "${pkgs.osmoda-keyd}/bin/osmoda-keyd"
          "--socket ${cfg.keyd.socketPath}"
          "--data-dir ${cfg.stateDir}/keyd"
          "--policy-file ${cfg.keyd.policyFile}"
          "--agentd-socket ${cfg.agentd.socketPath}"
        ];
        Restart = "always";
        RestartSec = 3;
        RuntimeDirectory = "osmoda";
        StateDirectory = "osmoda";

        # Maximum isolation — zero network access
        PrivateNetwork = true;
        RestrictAddressFamilies = "AF_UNIX";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectKernelTunables = true;
        ProtectClock = true;
        ProtectHostname = true;
        PrivateDevices = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        ReadWritePaths = [ "${cfg.stateDir}/keyd" "/run/osmoda" ];
      };
    };

    # ===== WATCH DAEMON (SafeSwitch + Autopilot) =====
    systemd.services.osmoda-watch = mkIf cfg.watch.enable {
      description = "osModa Watch Daemon (SafeSwitch + autopilot)";
      wantedBy = [ "multi-user.target" ];
      after = [ "osmoda-agentd.service" ];
      requires = [ "osmoda-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = concatStringsSep " " [
          "${pkgs.osmoda-watch}/bin/osmoda-watch"
          "--socket ${cfg.watch.socketPath}"
          "--agentd-socket ${cfg.agentd.socketPath}"
          "--data-dir ${cfg.stateDir}/watch"
          "--check-interval ${toString cfg.watch.checkInterval}"
        ];
        Restart = "always";
        RestartSec = 3;
        RuntimeDirectory = "osmoda";
        StateDirectory = "osmoda";
        # Runs as root — needs to execute nixos-rebuild, systemctl
      };
    };

    # ===== ROUTINES ENGINE =====
    systemd.services.osmoda-routines = mkIf cfg.routines.enable {
      description = "osModa Routines Engine (background automation)";
      wantedBy = [ "multi-user.target" ];
      after = [ "osmoda-agentd.service" ];
      requires = [ "osmoda-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = concatStringsSep " " [
          "${pkgs.osmoda-routines}/bin/osmoda-routines"
          "--socket ${cfg.routines.socketPath}"
          "--agentd-socket ${cfg.agentd.socketPath}"
          "--routines-dir ${cfg.routines.routinesDir}"
        ];
        Restart = "always";
        RestartSec = 3;
        RuntimeDirectory = "osmoda";
        StateDirectory = "osmoda";
        ProtectKernelTunables = true;
        LockPersonality = true;
      };
    };

    # ===== VOICE DAEMON =====
    systemd.services.osmoda-voice = mkIf cfg.voice.enable {
      description = "osModa Voice Daemon (STT + TTS)";
      wantedBy = [ "multi-user.target" ];
      after = [ "osmoda-agentd.service" "pipewire.service" ];
      requires = [ "osmoda-agentd.service" "pipewire.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = concatStringsSep " " [
          "${pkgs.osmoda-voice}/bin/osmoda-voice"
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
        RuntimeDirectory = "osmoda";
        StateDirectory = "osmoda";
      };

      environment = {
        OSMODA_SOCKET = cfg.agentd.socketPath;
      };
    };

    # ===== FIREWALL =====
    networking.firewall = {
      enable = true;
      allowedTCPPorts = [ ]
        ++ optionals cfg.ui.enable [ cfg.openclaw.port ];
    };

    # ===== BACKUP TIMER =====
    systemd.services.osmoda-backup = {
      description = "osModa daily backup";
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${pkgs.curl}/bin/curl -s --unix-socket ${cfg.agentd.socketPath} http://localhost/backup/create -X POST";
      };
    };

    systemd.timers.osmoda-backup = {
      description = "osModa daily backup timer";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = "*-*-* 03:00:00";
        Persistent = true;
        RandomizedDelaySec = "15min";
      };
    };

    # ===== WORKSPACE SETUP =====
    system.activationScripts.osmoda-workspace = ''
      mkdir -p ${cfg.stateDir}/{workspace,ledger,artifacts,memory,voice/models,voice/cache,keyd/keys,watch,routines,secrets}
      mkdir -p ${cfg.stateDir}/workspace/skills
      mkdir -p /var/backups/osmoda

      # Secure state directories
      chmod 700 ${cfg.stateDir}/keyd
      chmod 700 ${cfg.stateDir}/keyd/keys
      chmod 700 ${cfg.stateDir}/secrets
    '' + optionalString cfg.channels.whatsapp.enable ''
      # WhatsApp credential directory
      mkdir -p ${toString cfg.channels.whatsapp.credentialDir}
      chmod 700 ${toString cfg.channels.whatsapp.credentialDir}
    '' + ''

      # Create default keyd policy if not present
      if [ ! -f "${cfg.stateDir}/keyd/policy.json" ]; then
        cat > "${cfg.stateDir}/keyd/policy.json" << 'POLICY'
      {
        "rules": [
          {
            "action": "send",
            "max_amount": "1.0",
            "period": "daily",
            "allowed_destinations": null,
            "chain": "ethereum",
            "max_per_day": 10
          },
          {
            "action": "send",
            "max_amount": "10.0",
            "period": "daily",
            "allowed_destinations": null,
            "chain": "solana",
            "max_per_day": 20
          },
          {
            "action": "sign",
            "max_amount": null,
            "period": "daily",
            "allowed_destinations": null,
            "chain": null,
            "max_per_day": 100
          }
        ]
      }
      POLICY
        chmod 600 "${cfg.stateDir}/keyd/policy.json"
      fi

      # Symlink system skills from Nix store if package exists
      if [ -d "${pkgs.osmoda-system-skills or ""}/skills" ]; then
        for skill in ${pkgs.osmoda-system-skills or ""}/skills/*; do
          ln -sf "$skill" ${cfg.stateDir}/workspace/skills/$(basename "$skill") 2>/dev/null || true
        done
      fi

      # Install default templates if not present
      for tmpl in AGENTS.md SOUL.md TOOLS.md IDENTITY.md HEARTBEAT.md; do
        src="${./../../templates}/$tmpl"
        dst="${cfg.stateDir}/workspace/$tmpl"
        if [ -f "$src" ] && [ ! -f "$dst" ]; then
          cp "$src" "$dst"
        fi
      done
    '';
  };
}

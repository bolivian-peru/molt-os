# osModa Shell — Kiosk-mode desktop shell
# Sway in full-screen kiosk mode with Firefox pointed at OpenClaw Gateway.
# The conversation IS the desktop. Minimal chrome, maximum focus.
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.osmoda.shell;
  gatewayPort = config.services.osmoda.openclaw.port;

  # Sway kiosk configuration — full-screen chat, no desktop metaphors
  swayKioskConfig = pkgs.writeText "sway-kiosk-config" ''
    # === osModa Kiosk Shell ===

    set $mod Mod4

    # No window decorations — the chat IS the desktop
    default_border none
    default_floating_border none
    gaps inner 0
    gaps outer 0
    titlebar_padding 0
    font pango:monospace 0

    # Dark background
    output * bg #0a0a0f solid_color

    # Wait for gateway/setup wizard to be ready, then launch Firefox
    exec bash -c 'for i in $(seq 1 30); do curl -sf http://localhost:${toString gatewayPort}/ >/dev/null 2>&1 && break; sleep 1; done; exec ${cfg.browser}/bin/firefox --kiosk http://localhost:${toString gatewayPort}'

    # Launch Waybar (if useWaybar is enabled, swaybar block below is hidden)
    ${if cfg.useWaybar then "exec ${pkgs.waybar}/bin/waybar" else ""}

    # === Emergency keybinds ===
    # Super+T: open a terminal (escape hatch for power users)
    bindsym $mod+t exec ${pkgs.foot}/bin/foot
    # Super+Q: quit current window
    bindsym $mod+q kill
    # Super+Shift+E: exit Sway (emergency)
    bindsym $mod+Shift+e exec swaynag -t warning -m 'Exit osModa?' -B 'Yes' 'swaymsg exit'
    # Super+F: open file manager
    bindsym $mod+f exec ${pkgs.pcmanfm}/bin/pcmanfm

    # Window management (minimal — most users won't need this)
    bindsym $mod+Left focus left
    bindsym $mod+Right focus right
    bindsym $mod+Up focus up
    bindsym $mod+Down focus down
    bindsym $mod+Shift+f fullscreen toggle

    # Move windows (for multi-window scenarios)
    bindsym $mod+Shift+Left move left
    bindsym $mod+Shift+Right move right

    # Workspaces (hidden but available)
    bindsym $mod+1 workspace number 1
    bindsym $mod+2 workspace number 2
    bindsym $mod+Shift+1 move container to workspace number 1
    bindsym $mod+Shift+2 move container to workspace number 2

    # Fallback status bar (only used if Waybar is disabled)
    ${if cfg.useWaybar then "" else ''
    bar {
      position top
      height 28
      status_command while echo "$(date +'%H:%M  %b %d')"; do sleep 30; done
      pango_markup enabled
      font pango:JetBrains Mono 10
      colors {
        background #0a0a0f
        statusline #888899
        separator #333344
        focused_workspace #0a0a0f #0a0a0f #ffffff
        inactive_workspace #0a0a0f #0a0a0f #555566
      }
    }
    ''}

    # Input configuration
    input type:keyboard {
      xkb_options ctrl:nocaps
    }

    # Hide cursor after 3 seconds of inactivity (kiosk mode)
    seat seat0 hide_cursor 3000
  '';

  # Waybar config for richer status bar (optional, used if waybar is enabled)
  waybarConfig = builtins.toJSON {
    layer = "top";
    position = "top";
    height = 28;
    spacing = 0;
    modules-left = [];
    modules-center = [ "clock" ];
    modules-right = [ "pulseaudio" "network" "battery" ];
    clock = {
      format = "{:%H:%M}";
      format-alt = "{:%Y-%m-%d %H:%M}";
      tooltip-format = "{:%A, %B %d, %Y}";
    };
    battery = {
      format = "{icon} {capacity}%";
      format-icons = [ "▁" "▂" "▃" "▄" "▅" "▆" "▇" "█" ];
    };
    network = {
      format-wifi = "WiFi";
      format-ethernet = "ETH";
      format-disconnected = "Offline";
    };
    pulseaudio = {
      format = "Vol {volume}%";
      format-muted = "Muted";
    };
  };

  waybarStyle = ''
    * {
      font-family: "JetBrains Mono", monospace;
      font-size: 12px;
      color: #888899;
    }
    window#waybar {
      background: #0a0a0f;
      border: none;
    }
    #clock, #battery, #network, #pulseaudio {
      padding: 0 12px;
    }
  '';
in {
  options.services.osmoda.shell = {
    enable = mkEnableOption "osModa kiosk shell (Sway + Firefox fullscreen)";

    browser = mkOption {
      type = types.package;
      default = pkgs.firefox;
      description = "Browser package for the kiosk UI";
    };

    useWaybar = mkOption {
      type = types.bool;
      default = true;
      description = "Use Waybar instead of swaybar for richer status info";
    };

    fileManager = mkOption {
      type = types.bool;
      default = true;
      description = "Include a graphical file manager (accessible via Super+F)";
    };
  };

  config = mkIf cfg.enable {
    # Sway with kiosk config
    programs.sway = {
      enable = true;
      wrapperFeatures.gtk = true;
      extraPackages = with pkgs; [
        wl-clipboard
        foot          # lightweight terminal (emergency access)
        grim          # screenshot
      ] ++ optionals cfg.useWaybar [ waybar ]
        ++ optionals cfg.fileManager [ pcmanfm ];
    };

    # Auto-login via greetd
    services.greetd = {
      enable = true;
      settings = {
        default_session = {
          command = "${pkgs.sway}/bin/sway --config ${swayKioskConfig}";
          user = "agent";
        };
      };
    };

    # Waybar config (if enabled)
    environment.etc = mkIf cfg.useWaybar {
      "xdg/waybar/config".text = waybarConfig;
      "xdg/waybar/style.css".text = waybarStyle;
    };

    # System packages needed for the shell
    environment.systemPackages = with pkgs; [
      firefox
      foot
      wl-clipboard
      curl              # needed for readiness check before browser launch
      networkmanagerapplet  # WiFi management (nm-applet in tray)
    ] ++ optionals cfg.fileManager [ pcmanfm ];
  };
}

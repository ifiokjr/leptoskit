# Isolated Linux desktop with the real KDE AT-SPI driver and compositor.
FROM ubuntu:26.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake ninja-build git ca-certificates pkg-config \
    extra-cmake-modules qt6-base-dev qt6-base-private-dev qt6-wayland-dev \
    libkf6windowsystem-dev libkf6coreaddons-dev libwayland-dev kwayland-dev \
    libkpipewire-dev plasma-wayland-protocols libxkbcommon-dev \
    kwin-wayland kwin-common libcap2-bin dbus-x11 at-spi2-core gobject-introspection \
    python3-pip python3-venv python3-pyatspi python3-gi python3-flask python3-lxml python3-numpy \
    gir1.2-gtk-3.0 ruby pipewire wireplumber fonts-dejavu-core procps \
    libgtk-4-1 xdg-desktop-portal xdg-desktop-portal-kde zenity \
    && rm -rf /var/lib/apt/lists/*
# The virtual compositor needs no realtime scheduling. Ubuntu's file capability
# otherwise prevents exec inside Docker's default capability bounding set.
RUN if [ -n "$(getcap /usr/bin/kwin_wayland)" ]; then setcap -r /usr/bin/kwin_wayland; fi
RUN git clone https://github.com/KDE/selenium-webdriver-at-spi.git /opt/atspi \
    && git -C /opt/atspi checkout cb67767e0206f495f24ca98214912692982bd23c \
    && cmake -S /opt/atspi -B /opt/atspi/build -G Ninja -DBUILD_TESTING=OFF -DCMAKE_INSTALL_PREFIX=/usr/local \
    && cmake --build /opt/atspi/build --parallel 2 \
    && cmake --install /opt/atspi/build \
    && python3 -m venv --system-site-packages /opt/driver-venv \
    && /opt/driver-venv/bin/pip install -r /usr/local/share/selenium-webdriver-at-spi/requirements.txt
ENV PATH="/opt/driver-venv/bin:${PATH}" LANG=C.UTF-8 PYTHONUNBUFFERED=1
# XDG_CURRENT_DESKTOP makes xdg-desktop-portal pick its KDE backend, matching
# the KWin session. The dialog plugin presents file dialogs through the portal
# and message boxes through zenity, so both run inside the driver's session.
ENV LIBGL_ALWAYS_SOFTWARE=1 KWIN_COMPOSE=O2 QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1 \
    TEST_WITH_VIDEO_RECORDER=0 USE_CUSTOM_BUS=1 \
    XDG_CURRENT_DESKTOP=KDE \
    XDG_RUNTIME_DIR=/tmp/leptoskit-runtime FLASK_PORT=4723
# D-Bus authenticates the host UID used for the mounted artifact directory.
ARG NATIVE_UID=1000
ARG NATIVE_GID=1000
RUN if ! getent group "$NATIVE_GID"; then groupadd -g "$NATIVE_GID" native-test; fi \
    && if ! getent passwd "$NATIVE_UID"; then useradd -m -u "$NATIVE_UID" -g "$NATIVE_GID" native-test; fi
WORKDIR /workspace
CMD ["bash", "setup/scripts/native_linux_run.sh"]

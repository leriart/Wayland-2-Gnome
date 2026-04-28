# Phase 4: Proxy Selectivo con Passthrough Byte-Level

## Problema
cava-bg usa `wgpu::SurfaceTargetUnsafe::RawHandle` con punteros raw de `wl_display` y `wl_surface`. EGL internamente abre el socket Wayland desde el `wl_display_ptr` y espera un compositor real con `wl_drm`. El bridge (Phase 3b) implementa `wayland_server` que intercepta mensajes pero NO expone `wl_drm` ni permite que EGL funcione.

## Solución: Proxy Selectivo
El bridge opera en **dos modos simultáneos**:

1. **Intercepción (layer-shell):** El bridge intercepta mensajes `zwlr_layer_shell_v1` entrantes y los traduce a llamadas reales a Hyprland
2. **Passthrough (todo lo demás):** El bridge simplemente reenvía bytes entre el cliente y Hyprland para TODO excepto `wlr-layer-shell`

### Arquitectura
```
┌────────────┐  wayland-bridge-0  ┌──────────────────────┐  wayland-1  ┌──────────┐
│  cava-bg   │ ──────────────────▶│  Proxy Selectivo     │─────────────▶│ Hyprland │
│            │◀──────────────────│  (passthrough+intercept)│◀────────────│          │
└────────────┘                   └──────────────────────┘              └──────────┘
```

- Los mensajes de `zwlr_layer_shell_v1` son interceptados y traducidos
- Los mensajes de `wl_surface`, `wl_drm`, `eglSwapBuffers`, etc. pasan directamente entre cava-bg y Hyprland

### Implementación
La implementación requiere trabajar a nivel de socket Unix, parseando mensajes Wayland para detectar requests de `zwlr_layer_shell_v1` y reenviar el resto.

### Alternativa más simple para Phase 4a
En vez de proxy selectivo completo, el bridge puede simplemente **exponer `wl_drm` como un global falso** usando manejo de mensajes crudos. wgpu/EGL necesitan:
1. `wl_drm` global (para saber que hay GPU)
2. `wl_drm.device` (para obtener un fd de DRM)
3. `wl_drm.create_buffer` → `wl_buffer` (para crear buffers compatibles)
4. `wl_drm.authenticate` (autenticar conexión DRM)

El bridge puede implementar `wl_drm` a bajo nivel reenviando al `wl_drm` de Hyprland.

## Recomendación
Empezar con **Phase 4a: Proxy selectivo raw** que hace passthrough byte-level de TODO excepto `zwlr_layer_shell_v1`. Esto evita toda la complejidad de EGL/drm y funciona con cualquier cliente.

## Estado actual
Phase 3b: Protocol-aware proxy con dispatchers de wayland_server y wayland_client. Funciona para protocolo layer-shell (get_layer_surface, set_size, etc.) pero wgpu/EGL fallan porque el bridge no implementa wl_drm.

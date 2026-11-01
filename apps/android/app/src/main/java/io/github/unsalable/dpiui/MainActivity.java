package io.github.unsalable.dpiui;

import android.Manifest;
import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.net.VpnService;
import android.os.Build;
import android.os.Bundle;
import android.view.View;
import android.view.Window;
import android.view.WindowInsetsController;
import android.view.WindowManager;

/**
 * Uygulamanin penceresi. Arayuzun tamami Rust'ta (Slint); bu sinif yalnizca Android'in etkinlik
 * uzerinden yaptigi isleri ustlenir: VPN izni penceresi, bildirim izni, sistem cubuklari.
 */
public class MainActivity extends NativeActivity {
    private static final int REQUEST_VPN = 1;
    private static final int REQUEST_NOTIFICATIONS = 2;

    /** Acik etkinlik (yoksa null); yalnizca ana is parcacigindan okunur. */
    static MainActivity current;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        Native.ensureInit(this);
        current = this;
        super.onCreate(savedInstanceState);
        // Pencere susu (DecorView) NativeActivity'nin onCreate'inde kurulur.
        edgeToEdge();
    }

    @Override
    protected void onDestroy() {
        if (current == this) {
            current = null;
        }
        super.onDestroy();
    }

    /** Arayuz sistem cubuklarinin altina kadar cizilir; bosluklari Slint birakir (safe-area-insets). */
    private void edgeToEdge() {
        Window window = getWindow();
        window.setStatusBarColor(Color.TRANSPARENT);
        window.setNavigationBarColor(Color.TRANSPARENT);
        if (Build.VERSION.SDK_INT >= 29) {
            window.setNavigationBarContrastEnforced(false);
        }
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(false);
        } else {
            window.getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }
        if (Build.VERSION.SDK_INT >= 28) {
            WindowManager.LayoutParams attrs = window.getAttributes();
            attrs.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            window.setAttributes(attrs);
        }
        applySystemBars(Bridge.nightMode());
    }

    /** Koyu temada acik, acik temada koyu durum/gezinme simgeleri. */
    void applySystemBars(boolean dark) {
        Window window = getWindow();
        if (Build.VERSION.SDK_INT >= 30) {
            WindowInsetsController controller = window.getInsetsController();
            if (controller != null) {
                int mask = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                        | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                controller.setSystemBarsAppearance(dark ? 0 : mask, mask);
            }
        } else {
            View decor = window.getDecorView();
            int light = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
            int flags = decor.getSystemUiVisibility();
            decor.setSystemUiVisibility(dark ? flags & ~light : flags | light);
        }
        // Arayuz cizilene kadar gorunen zemin, tema renginde.
        window.getDecorView().setBackgroundColor(dark ? Color.BLACK : 0xFFF2F2F7);
    }

    void requestVpnPermission() {
        Intent intent = VpnService.prepare(this);
        if (intent == null) {
            Native.onVpnPermission(true);
            return;
        }
        try {
            startActivityForResult(intent, REQUEST_VPN);
        } catch (ActivityNotFoundException e) {
            Native.onVpnPermission(false);
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode != REQUEST_VPN) {
            super.onActivityResult(requestCode, resultCode, data);
            return;
        }
        boolean granted = resultCode == RESULT_OK;
        // Bagliyken durum bildirimi gorunsun (Android 13+ ayrica sorar).
        if (granted && Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, REQUEST_NOTIFICATIONS);
        }
        Native.onVpnPermission(granted);
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        // Izinden once gonderilen on plan bildirimi gosterilmemisti; simdi yeniden gonderilir.
        if (requestCode == REQUEST_NOTIFICATIONS && DpiVpnService.instance != null) {
            DpiVpnService.instance.refreshNotification();
        }
    }
}

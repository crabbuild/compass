<?php

class Route {
    public static function get($path, $handler) {}
}

class OrdinaryController {
    public function show() {}
}

Route::get('/not-laravel', [OrdinaryController::class, 'show']);

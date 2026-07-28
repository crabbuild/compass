<?php

use Illuminate\Support\Facades\Route;

class UserController {
    public function index() {}
    public function store() {}
    public function destroy() {}
}

class PhotoController {
    public function index() {}
    public function create() {}
    public function store() {}
    public function show() {}
    public function edit() {}
    public function update() {}
    public function destroy() {}
}

Route::get('/users', [UserController::class, 'index']);
Route::post('/users', 'UserController@store');
Route::match(['get', 'post'], '/users/search', [UserController::class, 'index']);

Route::prefix('/admin')->group(function () {
    Route::delete('/users/{id}', [UserController::class, 'destroy']);
});

Route::resource('/photos', PhotoController::class);

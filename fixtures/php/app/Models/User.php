<?php

namespace App\Models;

class User extends Model
{
    public static function find(int $id): ?User
    {
        return static::query()->whereKey($id)->first();
    }

    public function displayName(): string
    {
        return ucfirst($this->name);
    }
}

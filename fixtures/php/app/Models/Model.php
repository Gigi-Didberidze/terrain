<?php

namespace App\Models;

abstract class Model
{
    public static function query(): Builder
    {
        return new Builder(static::class);
    }
}

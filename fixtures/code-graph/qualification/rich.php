<?php

trait Logs
{
    public function log(string $message): void {}
}

class Worker
{
    use Logs;

    public function run(): void
    {
        $callback = function (): void {
            $this->log('working');
        };
        $callback();
    }
}

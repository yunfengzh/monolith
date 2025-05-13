Monolith is a framework based on tokio and bevy.

Later pseudo-code gives an overview:

```plaintext
async sprite::run() {
    sprite_in_village();
    ...
    sprite_enter_dungeon();
    loop {
        match evt = from_bevy().await {
            GameEvent::MoveLeft => {
                self.pos.translation.x -= 1.0;
                to_bevy().send(Task::SyncEntity, self.entity, self.pos);
            }
            ...
        }
    }
    ...
}
```

1. Sprite::run should be based on sprite state machine rather than being split to fit with ECS.
2. Sprite::run hardcodes GameEvent (logic event).
3. Monolith treats bevy as a display and peripheral wrapper library -- from_bevy and to_bevy.
4. Monolith supports tokio, that is, you can run time-cost algorithm in coroutine context.

## More highlight about monolith

- DeveloperBackend: permit developer inject GameEvent, observe inner variables etc.
- RecordBackend/ReplayBackend: game recording feature.
- Referee-Athlete: a brand-new lock for savefile. 

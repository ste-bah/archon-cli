impl MemoryTrait for OwnershipRaceMemory {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        self.inner.store_memory(
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
    }

    fn store_memory_with_id_outcome(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<StoreMemoryOutcome, MemoryError> {
        if id == self.correction_id {
            self.preflight_barrier.wait();
        }
        let outcome = self.inner.store_memory_with_id_outcome(
            id,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )?;
        if id == self.correction_id && outcome.created {
            let mut creator = self
                .creator_thread
                .lock()
                .expect("creator thread poisoned");
            *creator = Some(std::thread::current().id());
            self.creator_ready.notify_all();
        }
        Ok(outcome)
    }

    fn store_memory_with_id(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<Memory, MemoryError> {
        self.store_memory_with_id_outcome(
            id,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
        .map(|outcome| outcome.memory)
    }

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        self.inner.get_memory(id)
    }

    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        self.inner.inspect_memory(id)
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        self.inner.update_memory(id, content, tags)
    }

    fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        let updated = self.inner.apply_importance_delta(id, delta, provenance_id)?;
        if self.boost_turn.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            let mut completed = self
                .creator_finished_boost
                .lock()
                .expect("boost completion poisoned");
            *completed = true;
            self.creator_boosted.notify_one();
        }
        Ok(updated)
    }

    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        self.inner
            .has_importance_application(memory_id, provenance_id)
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        self.inner.delete_memory(id)
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        self.inner
            .create_relationship(from_id, to_id, rel_type, context, strength)?;
        if from_id == self.correction_id {
            let creator = self
                .creator_ready
                .wait_while(
                    self.creator_thread.lock().expect("creator thread poisoned"),
                    |thread_id| thread_id.is_none(),
                )
                .expect("creator thread poisoned");
            let is_creator = creator.as_ref() == Some(&std::thread::current().id());
            drop(creator);
            if !is_creator {
                let boosted = self
                    .creator_finished_boost
                    .lock()
                    .expect("boost completion poisoned");
                drop(
                    self.creator_boosted
                        .wait_while(boosted, |done| !*done)
                        .expect("boost completion poisoned"),
                );
                return Err(MemoryError::Database(
                    "injected non-owner failure after relationship".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        self.inner.recall_memories(query, limit)
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        self.inner.search_memories(filter)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        self.inner.list_recent(limit)
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        self.inner.memory_count()
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        self.inner.clear_all()
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        self.inner.get_related_memories(id, depth)
    }
}

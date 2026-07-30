import org.springframework.scheduling.annotation.Scheduled;

class CleanupJobs {
    @Scheduled(cron = "0 0 * * * *")
    public void cleanup() {}
}

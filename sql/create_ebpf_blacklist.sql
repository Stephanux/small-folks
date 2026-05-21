CREATE TABLE IF NOT EXISTS ebpf_blacklist (
    id           INT AUTO_INCREMENT PRIMARY KEY,
    ip_address   VARCHAR(15)  NOT NULL,
    blocked_at   DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    unblock_at   DATETIME     DEFAULT NULL,
    unblocked_at DATETIME     DEFAULT NULL,
    reason       VARCHAR(100) NOT NULL DEFAULT 'rate_limit_exceeded',
    UNIQUE KEY uq_ip_active (ip_address, unblocked_at),
    INDEX idx_ip      (ip_address),
    INDEX idx_blocked (blocked_at),
    INDEX idx_unblock (unblock_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

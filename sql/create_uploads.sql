CREATE TABLE uploads (
    id          INT AUTO_INCREMENT PRIMARY KEY,
    uuid        CHAR(36)     NOT NULL UNIQUE,
    filename    VARCHAR(255) NOT NULL,   -- nom original
    stored_as   VARCHAR(255) NOT NULL,   -- uuid.ext sur disque
    mime_type   VARCHAR(100) NOT NULL,
    size_bytes  BIGINT       NOT NULL,
    upload_dir  VARCHAR(255) NOT NULL,
    created_at  DATETIME     DEFAULT CURRENT_TIMESTAMP
);